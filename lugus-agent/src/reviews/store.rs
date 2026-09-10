use super::*;
use chrono::{DateTime, Utc};

/// Each mutation is atomic. Implementations must fence attempts and compare revisions.
/// Caller-supplied IDs are stable operation keys, not runtime tool-call IDs.
pub trait ReviewStore: Send + Sync {
    fn save_thesis(
        &self,
        id: &str,
        expected_revision: u64,
        text: &str,
        now: DateTime<Utc>,
    ) -> ReviewResult<ThesisRevision>;
    fn thesis(&self, id: &str, revision: Option<u64>) -> ReviewResult<ThesisRevision>;
    fn enqueue(
        &self,
        id: &str,
        thesis_id: &str,
        revision: u64,
        evidence: Vec<Evidence>,
        now: DateTime<Utc>,
    ) -> ReviewResult<Review>;
    fn review(&self, id: &str) -> ReviewResult<Review>;
    fn claim(&self, id: &str, runtime_identity: &str, now: DateTime<Utc>) -> ReviewResult<Attempt>;
    fn submit(
        &self,
        attempt: &Attempt,
        draft: AssessmentDraft,
        now: DateTime<Utc>,
    ) -> ReviewResult<Assessment>;
    fn finish(
        &self,
        attempt: &Attempt,
        status: ReviewStatus,
        detail: &str,
        now: DateTime<Utc>,
    ) -> ReviewResult<Review>;
    /// Startup only, after all previous executors have stopped. Opening is not recovery.
    fn recover(&self, now: DateTime<Utc>) -> ReviewResult<usize>;
    fn assessment(&self, id: &str) -> ReviewResult<Assessment>;
    fn assessments(&self, thesis_id: &str) -> ReviewResult<Vec<Assessment>>;
}
