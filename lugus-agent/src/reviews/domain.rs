use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub type ReviewResult<T> = Result<T, ReviewError>;
#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    #[error("invalid review input: {0}")]
    Invalid(String),
    #[error("review conflict: {0}")]
    Conflict(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("review storage: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("review serialization: {0}")]
    Json(#[from] serde_json::Error),
    #[error("financial evidence: {0}")]
    Financial(String),
    #[error("review store lock poisoned")]
    Poisoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThesisRevision {
    pub thesis_id: String,
    pub revision: u64,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    pub source_ref: String,
    pub title: String,
    pub content: Value,
}
impl Evidence {
    pub fn new(source_ref: &str, title: &str, content: Value) -> ReviewResult<Self> {
        let id = evidence_hash(source_ref, title, &content)?;
        let result = Self {
            id,
            source_ref: source_ref.into(),
            title: title.into(),
            content,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> ReviewResult<()> {
        text(&self.source_ref, "evidence source", 2048)?;
        text(&self.title, "evidence title", 1024)?;
        if serde_json::to_vec(self)?.len() > 65536 {
            return invalid("evidence exceeds 64 KiB");
        }
        if self.id != evidence_hash(&self.source_ref, &self.title, &self.content)? {
            return invalid("evidence hash does not match payload");
        }
        Ok(())
    }
}
fn evidence_hash(source: &str, title: &str, content: &Value) -> ReviewResult<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(source, title, content))?)
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Queued,
    Running,
    Completed,
    Interrupted,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub id: String,
    pub thesis: ThesisRevision,
    pub previous_assessment_id: Option<String>,
    pub evidence: Vec<Evidence>,
    pub status: ReviewStatus,
    pub attempt: u64,
    pub runtime_identity: Option<String>,
    pub requested_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub detail: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub review_id: String,
    pub number: u64,
}
impl Attempt {
    pub fn run_id(&self) -> String {
        format!("{}:{}", self.review_id, self.number)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceClaim {
    pub text: String,
    pub evidence_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssessmentDraft {
    pub interpretation: String,
    pub conclusion: String,
    pub supporting: Vec<EvidenceClaim>,
    pub opposing: Vec<EvidenceClaim>,
    pub uncertainty: Vec<String>,
    pub open_questions: Vec<String>,
    pub changes: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assessment {
    pub id: String,
    pub review_id: String,
    pub thesis_id: String,
    pub thesis_revision: u64,
    pub previous_assessment_id: Option<String>,
    pub evidence_ids: Vec<String>,
    pub attempt: u64,
    pub runtime_identity: String,
    pub created_at: DateTime<Utc>,
    pub draft: AssessmentDraft,
}

pub fn validate_assessment(draft: &AssessmentDraft, evidence: &[Evidence]) -> ReviewResult<()> {
    if serde_json::to_vec(draft)?.len() > 32768 {
        return invalid("assessment exceeds 32 KiB");
    }
    for (label, value) in [
        ("interpretation", &draft.interpretation),
        ("conclusion", &draft.conclusion),
        ("changes", &draft.changes),
    ] {
        text(value, label, 32768)?;
    }
    let allowed: HashSet<_> = evidence.iter().map(|e| e.id.as_str()).collect();
    for claim in draft.supporting.iter().chain(&draft.opposing) {
        text(&claim.text, "claim", 32768)?;
        if claim.evidence_ids.is_empty()
            || claim
                .evidence_ids
                .iter()
                .any(|id| !allowed.contains(id.as_str()))
        {
            return invalid("claims must cite selected evidence");
        }
    }
    for value in draft.uncertainty.iter().chain(&draft.open_questions) {
        text(value, "question or uncertainty", 32768)?;
    }
    Ok(())
}
pub(super) fn validate_evidence(items: &[Evidence]) -> ReviewResult<()> {
    if items.len() > 64 {
        return invalid("select at most 64 evidence items");
    }
    let mut ids = HashSet::new();
    for e in items {
        e.validate()?;
        if !ids.insert(&e.id) {
            return invalid("duplicate evidence ID");
        }
    }
    Ok(())
}
pub(super) fn text(value: &str, label: &str, max: usize) -> ReviewResult<()> {
    if value.trim().is_empty() || value.len() > max {
        return invalid(&format!("{label} must be nonempty and at most {max} bytes"));
    }
    Ok(())
}
pub(super) fn invalid<T>(message: &str) -> ReviewResult<T> {
    Err(ReviewError::Invalid(message.into()))
}
pub(super) fn conflict<T>(message: &str) -> ReviewResult<T> {
    Err(ReviewError::Conflict(message.into()))
}
