//! Immutable source retrievals with exact, provider-scoped bounded reads.
use super::{
    SqliteRepository,
    bounded::{BoundedReadError, ReadLimits, ReadResult},
};
use crate::{
    domain::{ProviderIdentity, fingerprint},
    error::{Error, ErrorKind, Result},
    instruments::{InstrumentLookup, InstrumentMetadata, InstrumentObservation, validate_provider},
};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};

pub trait InstrumentRepository {
    fn save_instrument_observation(
        &mut self,
        provider: &ProviderIdentity,
        request: &InstrumentLookup,
        metadata: &InstrumentMetadata,
    ) -> Result<InstrumentObservation>;
}
impl InstrumentRepository for SqliteRepository {
    fn save_instrument_observation(
        &mut self,
        provider: &ProviderIdentity,
        request: &InstrumentLookup,
        metadata: &InstrumentMetadata,
    ) -> Result<InstrumentObservation> {
        validate_provider(provider)?;
        metadata.validate_for(request)?;
        let recorded_at = Utc::now();
        self.connection.execute(
            "INSERT INTO instrument_observations(provider_id,provider,request,payload,recorded_at) VALUES(?1,?2,?3,?4,?5)",
            params![fingerprint(provider)?, serde_json::to_string(provider)?, serde_json::to_string(request)?, serde_json::to_string(metadata)?, recorded_at.to_rfc3339()],
        )?;
        Ok(InstrumentObservation {
            id: self.connection.last_insert_rowid(),
            provider: provider.clone(),
            request: request.clone(),
            metadata: metadata.clone(),
            recorded_at,
        })
    }
}
impl SqliteRepository {
    /// Bounds stored UTF-8 provider/request/payload/timestamp bytes plus the i64 ID
    /// before decoding, on one SQLite snapshot. Does not materialize other rows.
    pub fn bounded_instrument_observation(
        &self,
        provider: &ProviderIdentity,
        id: i64,
        limits: ReadLimits,
    ) -> ReadResult<InstrumentObservation> {
        limits.validate()?;
        validate_provider(provider)?;
        if id <= 0 {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "positive instrument observation ID required",
            )
            .into());
        }
        let tx = self.connection.unchecked_transaction()?;
        let key = fingerprint(provider)?;
        let bytes: i64 = tx.query_row(
            "SELECT length(CAST(provider AS BLOB))+length(CAST(request AS BLOB))+length(CAST(payload AS BLOB))+length(CAST(recorded_at AS BLOB))+8 FROM instrument_observations WHERE id=?1 AND provider_id=?2",
            params![id, key], |row| row.get(0),
        ).optional()?.ok_or_else(|| Error::new(ErrorKind::NotFound, "instrument observation not found for provider"))?;
        if bytes < 0 || bytes as u64 > limits.max_bytes as u64 {
            return Err(BoundedReadError::LimitExceeded);
        }
        let (stored_provider, request, payload, recorded_at): (String, String, String, String) = tx.query_row(
            "SELECT provider,request,payload,recorded_at FROM instrument_observations WHERE id=?1 AND provider_id=?2", params![id, key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        // Deserialize the provider through the same strict boundary as observation JSON.
        let mut decoder = serde_json::Deserializer::from_str(&stored_provider);
        let stored_provider =
            crate::instruments::provider_identity(&mut decoder).map_err(Error::from)?;
        decoder.end().map_err(Error::from)?;
        let request: InstrumentLookup = serde_json::from_str(&request).map_err(Error::from)?;
        let metadata: InstrumentMetadata = serde_json::from_str(&payload).map_err(Error::from)?;
        if stored_provider != *provider || metadata.validate_for(&request).is_err() {
            return Err(Error::new(
                ErrorKind::Persistence,
                "invalid stored instrument evidence or provider",
            )
            .into());
        }
        let result = InstrumentObservation {
            id,
            provider: stored_provider,
            request,
            metadata,
            recorded_at: recorded_at.parse().map_err(|_| {
                Error::new(
                    ErrorKind::Persistence,
                    "invalid stored instrument recording timestamp",
                )
            })?,
        };
        tx.commit()?;
        Ok(result)
    }
}
