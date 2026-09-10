//! Bounds the legacy catalog conflict check without changing its identity rules.
use super::{
    SqliteRepository,
    bounded::{BoundedReadError, ReadLimits, ReadResult},
};
use crate::{
    error::{Error, ErrorKind},
    resolution::catalog::{CatalogRepository, ResolutionOutcome},
};
use rusqlite::{OptionalExtension, params};

impl SqliteRepository {
    /// The current conflict policy can consult every historical catalog retrieval.
    /// Conservatively bound that complete input before invoking it. A large catalog
    /// returns LimitExceeded, never a truncated history that could hide conflicts.
    pub fn bounded_resolution_outcome(
        &self,
        run: i64,
        limits: ReadLimits,
    ) -> ReadResult<ResolutionOutcome> {
        limits.validate()?;
        if run <= 0 {
            return Err(Error::new(ErrorKind::InvalidRequest, "positive run ID required").into());
        }
        let transaction = self.connection.unchecked_transaction()?;
        let metadata:i64=transaction.query_row(
            "SELECT length(CAST(request AS BLOB))+length(CAST(status AS BLOB))+coalesce(length(CAST(snapshot AS BLOB)),0)+coalesce(length(CAST(coverage AS BLOB)),0)+coalesce(length(CAST(error AS BLOB)),0)+coalesce(length(CAST(failure AS BLOB)),0) FROM resolution_runs WHERE id=?1",
            [run],|row|row.get(0)).optional()?.ok_or_else(||Error::new(ErrorKind::NotFound,"resolution run not found"))?;
        let row_limit = limits.max_items as i64;
        let (count,bytes):(i64,i64)=transaction.query_row(
            "SELECT count(*),coalesce(sum(size),0) FROM (SELECT length(CAST(o.payload AS BLOB))+length(CAST(o.provider AS BLOB))+length(CAST(r.recorded_at AS BLOB))+length(CAST(r.retrieved_at AS BLOB))+length(CAST(r.match_reasons AS BLOB)) AS size FROM catalog_retrievals r JOIN catalog_observations o ON o.id=r.observation_id JOIN resolution_pages p ON p.id=r.page_id LIMIT ?1)",
            params![row_limit.saturating_add(1)],|row|Ok((row.get(0)?,row.get(1)?)))?;
        let bytes = metadata
            .checked_add(bytes)
            .ok_or(BoundedReadError::LimitExceeded)?;
        if count > row_limit || bytes < 0 || bytes as u64 > limits.max_bytes as u64 {
            return Err(BoundedReadError::LimitExceeded);
        }
        let result = self.resolution_outcome(run)?;
        transaction.commit()?;
        Ok(result)
    }
}
