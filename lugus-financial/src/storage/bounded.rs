//! Exact-run reads with preflight bounds in the same SQLite read transaction.
//!
//! Limits count stored UTF-8 payload/metadata bytes plus retrieval metadata, not
//! Rust allocator overhead or serialized output size. Callers must additionally
//! bound their own output. Unrelated provider runs are never materialized.
use super::{SqliteRepository, fingerprint, validate_identity};
use crate::{
    domain::ProviderIdentity,
    error::{Error, ErrorKind, Result},
    selection::{FinancialRunEvidence, MarketRunEvidence},
};
use rusqlite::{OptionalExtension, params};

#[derive(Debug, Clone, Copy)]
pub struct ReadLimits {
    pub max_items: usize,
    pub max_bytes: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum BoundedReadError {
    #[error("stored evidence exceeds read limits")]
    LimitExceeded,
    #[error(transparent)]
    Financial(#[from] Error),
}
impl From<rusqlite::Error> for BoundedReadError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Financial(error.into())
    }
}
pub type ReadResult<T> = std::result::Result<T, BoundedReadError>;

impl ReadLimits {
    fn validate(self) -> ReadResult<()> {
        if self.max_items == 0
            || self.max_bytes == 0
            || self.max_items > i64::MAX as usize
            || self.max_bytes > i64::MAX as usize
        {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "positive representable read limits required",
            )
            .into());
        }
        Ok(())
    }
}

impl SqliteRepository {
    pub fn repository_identity(&self) -> Result<String> {
        Ok(self.connection.query_row(
            "SELECT identity FROM repository_identity WHERE singleton=1",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn bounded_financial_run(
        &self,
        provider: &ProviderIdentity,
        run_id: i64,
        limits: ReadLimits,
    ) -> ReadResult<FinancialRunEvidence> {
        self.bounded_run(provider, run_id, limits, false, |repo| {
            repo.read_financial_runs(provider, Some(run_id))
        })
    }

    pub fn bounded_market_run(
        &self,
        provider: &ProviderIdentity,
        run_id: i64,
        limits: ReadLimits,
    ) -> ReadResult<MarketRunEvidence> {
        self.bounded_run(provider, run_id, limits, true, |repo| {
            repo.read_market_runs(provider, Some(run_id))
        })
    }

    fn bounded_run<T>(
        &self,
        provider: &ProviderIdentity,
        run_id: i64,
        limits: ReadLimits,
        market: bool,
        read: impl FnOnce(&Self) -> Result<Vec<T>>,
    ) -> ReadResult<T> {
        limits.validate()?;
        validate_identity(provider)?;
        if run_id <= 0 {
            return Err(Error::new(ErrorKind::InvalidRequest, "positive run ID required").into());
        }
        // Keep counts and decoding on one snapshot even if a provider commits a
        // new page concurrently. No provider/network effects occur in this scope.
        let transaction = self.connection.unchecked_transaction()?;
        let provider_key = fingerprint(provider)?;
        let metadata_sql = if market {
            "SELECT length(CAST(query AS BLOB))+coalesce(length(CAST(cursor AS BLOB)),0)+coalesce(length(CAST(coverage AS BLOB)),0)+coalesce(length(CAST(error AS BLOB)),0)+length(CAST(started_at AS BLOB))+coalesce(length(CAST(finished_at AS BLOB)),0) FROM market_runs WHERE id=?1 AND provider_id=?2"
        } else {
            "SELECT length(CAST(query AS BLOB))+length(CAST(operation AS BLOB))+coalesce(length(CAST(filings_cursor AS BLOB)),0)+coalesce(length(CAST(facts_cursor AS BLOB)),0)+coalesce(length(CAST(error AS BLOB)),0)+length(CAST(started_at AS BLOB))+coalesce(length(CAST(finished_at AS BLOB)),0) FROM runs WHERE id=?1 AND provider_id=?2"
        };
        let metadata: i64 = transaction
            .query_row(metadata_sql, params![run_id, provider_key], |r| r.get(0))
            .optional()?
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "run not found for provider"))?;
        let identity_bytes = serde_json::to_vec(provider).map_err(Error::from)?.len();
        let budget = limits
            .max_bytes
            .checked_sub(identity_bytes)
            .and_then(|n| n.checked_sub(usize::try_from(metadata).ok()?))
            .ok_or(BoundedReadError::LimitExceeded)?;
        // LIMIT bounds rows inspected for the application budget; aggregate
        // lengths in SQLite without retrieving or decoding payload strings.
        let rows_sql = if market {
            "SELECT count(*),coalesce(sum(size),0) FROM (SELECT length(CAST(o.payload AS BLOB))+length(CAST(o.fingerprint AS BLOB))+length(CAST(r.retrieved_at AS BLOB)) AS size FROM market_run_observations r JOIN price_observations o ON o.id=r.observation_id WHERE r.run_id=?1 LIMIT ?2)"
        } else {
            "SELECT count(*),coalesce(sum(size),0) FROM (SELECT length(CAST(o.payload AS BLOB))+length(CAST(o.fingerprint AS BLOB))+length(CAST(r.retrieved_at AS BLOB)) AS size FROM run_observations r JOIN observations o ON o.id=r.observation_id WHERE r.run_id=?1 LIMIT ?2)"
        };
        let row_limit =
            i64::try_from(limits.max_items).map_err(|_| BoundedReadError::LimitExceeded)?;
        let (count, bytes): (i64, i64) = transaction.query_row(
            rows_sql,
            params![run_id, row_limit.saturating_add(1)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if count > row_limit || bytes < 0 || bytes as u64 > budget as u64 {
            return Err(BoundedReadError::LimitExceeded);
        }
        let result = read(self)?
            .pop()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "run not found for provider"))?;
        transaction.commit()?;
        Ok(result)
    }
}
