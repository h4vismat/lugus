//! Narrow bounded association checks used by application-owned references.
use super::{
    DocumentObservation, SqliteRepository,
    bounded::{BoundedReadError, ReadLimits, ReadResult},
    fingerprint,
};
use crate::{
    domain::ProviderIdentity,
    error::{Error, ErrorKind},
    resolution::catalog::{CatalogEntry, CatalogRepository, CatalogSelection, ResolutionOutcome},
};
use rusqlite::{OptionalExtension, params};
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScopedResolutionEvidence {
    pub outcome: ResolutionOutcome,
    pub request: crate::resolution::SearchRequest,
    pub entries: Vec<CatalogEntry>,
}
impl SqliteRepository {
    pub fn bounded_repository_identity(&self, max_bytes: usize) -> ReadResult<String> {
        let tx = self.connection.unchecked_transaction()?;
        let length: i64 = tx.query_row(
            "SELECT length(CAST(identity AS BLOB)) FROM repository_identity WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        if length < 1 || length as u64 > max_bytes as u64 {
            return Err(BoundedReadError::LimitExceeded);
        }
        let result = self.repository_identity()?;
        tx.commit()?;
        Ok(result)
    }
    pub fn bounded_document(
        &self,
        observation: &DocumentObservation,
        max_bytes: usize,
    ) -> ReadResult<Vec<u8>> {
        if max_bytes == 0 {
            return Err(BoundedReadError::LimitExceeded);
        }
        let tx = self.connection.unchecked_transaction()?;
        let size:Option<i64>=tx.query_row("SELECT length(c.content) FROM document_content c WHERE c.checksum=?1 AND EXISTS(SELECT 1 FROM document_observations o WHERE o.checksum=c.checksum AND o.provider_id=?2 AND o.source_url=?3 AND o.media_type=?4 AND o.retrieved_at=?5)",params![observation.checksum,fingerprint(&observation.provider)?,observation.source_url,observation.media_type,observation.retrieved_at.to_rfc3339()],|r|r.get(0)).optional()?;
        let size =
            size.ok_or_else(|| Error::new(ErrorKind::NotFound, "document association not found"))?;
        if size < 0 || size as u64 > max_bytes as u64 {
            return Err(BoundedReadError::LimitExceeded);
        }
        // The entire original is bounded before loading so its checksum can be verified.
        let bytes = super::Repository::stored_document(self, &observation.checksum)?;
        tx.commit()?;
        Ok(bytes)
    }
    fn check_resolution_provider(
        &self,
        p: &ProviderIdentity,
        run: i64,
        limits: ReadLimits,
    ) -> ReadResult<()> {
        limits.validate()?;
        let size: i64 = self
            .connection
            .query_row(
                "SELECT length(CAST(provider AS BLOB)) FROM resolution_runs WHERE id=?1",
                [run],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "resolution run not found"))?;
        if size < 0 || size as u64 > limits.max_bytes as u64 {
            return Err(BoundedReadError::LimitExceeded);
        }
        let stored: String = self.connection.query_row(
            "SELECT provider FROM resolution_runs WHERE id=?1",
            [run],
            |r| r.get(0),
        )?;
        if serde_json::from_str::<ProviderIdentity>(&stored).map_err(Error::from)? != *p {
            return Err(Error::new(ErrorKind::NotFound, "resolution provider mismatch").into());
        }
        Ok(())
    }
    pub fn bounded_scoped_resolution(
        &self,
        p: &ProviderIdentity,
        run: i64,
        limits: ReadLimits,
    ) -> ReadResult<ScopedResolutionEvidence> {
        self.check_resolution_provider(p, run, limits)?;
        self.with_bounded_resolution(run, limits, |repo| {
            Ok(ScopedResolutionEvidence {
                outcome: repo.resolution_outcome(run)?,
                request: serde_json::from_str(&repo.connection.query_row(
                    "SELECT request FROM resolution_runs WHERE id=?1",
                    [run],
                    |r| r.get::<_, String>(0),
                )?)?,
                entries: repo.catalog_records(Some(run), None)?,
            })
        })
    }
    pub fn bounded_select_candidate(
        &mut self,
        p: &ProviderIdentity,
        run: i64,
        observation: i64,
        limits: ReadLimits,
    ) -> ReadResult<CatalogSelection> {
        // Existing candidate selection materializes one immutable retrieval plus run metadata.
        // Preflight the same bounded catalog policy before delegating its mutation.
        self.bounded_scoped_resolution(p, run, limits)?;
        Ok(CatalogRepository::select_candidate(self, run, observation)?)
    }
}
