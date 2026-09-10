use super::domain::{conflict, invalid, text, validate_evidence};
use super::*;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::de::DeserializeOwned;
use std::{path::Path, sync::Mutex};

pub struct SqliteReviewStore {
    connection: Mutex<Connection>,
}
impl SqliteReviewStore {
    pub fn open(path: impl AsRef<Path>) -> ReviewResult<Self> {
        let mut c = Connection::open(path)?;
        c.busy_timeout(std::time::Duration::from_secs(5))?;
        c.execute_batch("PRAGMA foreign_keys = ON;")?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let app: i64 = tx.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        if version == 0 && app == 0 {
            let tables: i64 = tx.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )?;
            if tables != 0 {
                return invalid("agent database must be separate from existing databases");
            }
            tx.execute_batch(include_str!("schema.sql"))?;
        } else if version != 1 || app != 1280651858 {
            return invalid("unsupported or unrelated agent database");
        }
        tx.commit()?;
        Ok(Self {
            connection: Mutex::new(c),
        })
    }
    fn read<T>(&self, f: impl FnOnce(&Connection) -> ReviewResult<T>) -> ReviewResult<T> {
        let c = self.connection.lock().map_err(|_| ReviewError::Poisoned)?;
        f(&c)
    }
    fn write<T>(&self, f: impl FnOnce(&Connection) -> ReviewResult<T>) -> ReviewResult<T> {
        let mut c = self.connection.lock().map_err(|_| ReviewError::Poisoned)?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = f(&tx)?;
        tx.commit()?;
        Ok(result)
    }
}
fn decode<T: DeserializeOwned>(payload: Option<String>, id: &str) -> ReviewResult<T> {
    Ok(serde_json::from_str(
        &payload.ok_or_else(|| ReviewError::NotFound(id.into()))?,
    )?)
}
fn thesis(c: &Connection, id: &str, revision: Option<u64>) -> ReviewResult<ThesisRevision> {
    let revision = revision
        .map(i64::try_from)
        .transpose()
        .map_err(|_| ReviewError::Invalid("revision exceeds SQLite range".into()))?;
    let payload = c.query_row("SELECT payload FROM thesis_revisions WHERE thesis_id=?1 AND (?2 IS NULL OR revision=?2) ORDER BY revision DESC LIMIT 1", params![id, revision], |r| r.get(0)).optional()?;
    decode(payload, id)
}
fn review(c: &Connection, id: &str) -> ReviewResult<Review> {
    decode(
        c.query_row("SELECT payload FROM reviews WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?,
        id,
    )
}
fn assessment(c: &Connection, id: &str) -> ReviewResult<Assessment> {
    decode(
        c.query_row("SELECT payload FROM assessments WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?,
        id,
    )
}
fn latest(c: &Connection, thesis_id: &str) -> ReviewResult<Option<String>> {
    Ok(c.query_row(
        "SELECT id FROM assessments WHERE thesis_id=?1 ORDER BY sequence DESC LIMIT 1",
        [thesis_id],
        |r| r.get(0),
    )
    .optional()?)
}
fn save_review(c: &Connection, review: &Review) -> ReviewResult<()> {
    let status = serde_json::to_value(review.status)?;
    c.execute(
        "UPDATE reviews SET status=?1,payload=?2 WHERE id=?3",
        params![status.as_str(), serde_json::to_string(review)?, review.id],
    )?;
    Ok(())
}
fn current(c: &Connection, r: &Review) -> ReviewResult<()> {
    if thesis(c, &r.thesis.thesis_id, None)?.revision != r.thesis.revision {
        return conflict("thesis changed; create a new review request");
    }
    if latest(c, &r.thesis.thesis_id)? != r.previous_assessment_id {
        return conflict("assessment history changed; create a new review request");
    }
    Ok(())
}
fn owns(r: &Review, a: &Attempt) -> ReviewResult<()> {
    if r.attempt != a.number || a.number == 0 {
        return conflict("attempt has been superseded");
    }
    Ok(())
}
impl ReviewStore for SqliteReviewStore {
    fn save_thesis(
        &self,
        id: &str,
        expected_revision: u64,
        value: &str,
        now: DateTime<Utc>,
    ) -> ReviewResult<ThesisRevision> {
        text(id, "thesis ID", 128)?;
        text(value, "thesis", 16384)?;
        self.write(|c| {
            let revision: i64 = c.query_row(
                "SELECT COALESCE(MAX(revision),0) FROM thesis_revisions WHERE thesis_id=?1",
                [id],
                |r| r.get(0),
            )?;
            if revision as u64 != expected_revision {
                return conflict("thesis revision changed");
            }
            let next = revision
                .checked_add(1)
                .ok_or_else(|| ReviewError::Invalid("revision overflow".into()))?;
            let result = ThesisRevision {
                thesis_id: id.into(),
                revision: next as u64,
                text: value.into(),
                created_at: now,
            };
            c.execute(
                "INSERT INTO thesis_revisions VALUES (?1,?2,?3)",
                params![id, next, serde_json::to_string(&result)?],
            )?;
            Ok(result)
        })
    }
    fn thesis(&self, id: &str, revision: Option<u64>) -> ReviewResult<ThesisRevision> {
        self.read(|c| thesis(c, id, revision))
    }
    fn enqueue(
        &self,
        id: &str,
        thesis_id: &str,
        revision: u64,
        evidence: Vec<Evidence>,
        now: DateTime<Utc>,
    ) -> ReviewResult<Review> {
        text(id, "review ID", 128)?;
        validate_evidence(&evidence)?;
        self.write(|c| {
            match review(c, id) {
                Ok(old) => {
                    if old.thesis.thesis_id == thesis_id
                        && old.thesis.revision == revision
                        && old.evidence == evidence
                    {
                        return Ok(old);
                    }
                    return conflict("review ID reused with different inputs");
                }
                Err(ReviewError::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
            let t = thesis(c, thesis_id, None)?;
            if t.revision != revision {
                return conflict("thesis revision changed");
            }
            let r = Review {
                id: id.into(),
                thesis: t,
                previous_assessment_id: latest(c, thesis_id)?,
                evidence,
                status: ReviewStatus::Queued,
                attempt: 0,
                runtime_identity: None,
                requested_at: now,
                updated_at: now,
                detail: None,
            };
            c.execute(
                "INSERT INTO reviews VALUES (?1,?2,?3,'queued',?4)",
                params![
                    id,
                    thesis_id,
                    r.thesis.revision as i64,
                    serde_json::to_string(&r)?
                ],
            )?;
            Ok(r)
        })
    }
    fn review(&self, id: &str) -> ReviewResult<Review> {
        self.read(|c| review(c, id))
    }
    fn claim(&self, id: &str, runtime_identity: &str, now: DateTime<Utc>) -> ReviewResult<Attempt> {
        text(runtime_identity, "runtime identity", 1024)?;
        self.write(|c| {
            let mut r = review(c, id)?;
            if matches!(r.status, ReviewStatus::Completed | ReviewStatus::Running) {
                return conflict("review is already running or complete");
            }
            current(c, &r)?;
            let running: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM reviews WHERE thesis_id=?1 AND status='running')",
                [&r.thesis.thesis_id],
                |r| r.get(0),
            )?;
            if running {
                return conflict("another review is running for this thesis");
            }
            r.attempt = r
                .attempt
                .checked_add(1)
                .ok_or_else(|| ReviewError::Invalid("attempt overflow".into()))?;
            r.status = ReviewStatus::Running;
            r.runtime_identity = Some(runtime_identity.into());
            r.updated_at = now;
            r.detail = None;
            save_review(c, &r)?;
            Ok(Attempt {
                review_id: id.into(),
                number: r.attempt,
            })
        })
    }
    fn submit(
        &self,
        attempt: &Attempt,
        draft: AssessmentDraft,
        now: DateTime<Utc>,
    ) -> ReviewResult<Assessment> {
        self.write(|c| {
            let mut r = review(c, &attempt.review_id)?;
            owns(&r, attempt)?;
            validate_assessment(&draft, &r.evidence)?;
            if r.status == ReviewStatus::Completed {
                let saved = assessment(c, &r.id)?;
                return if saved.draft == draft {
                    Ok(saved)
                } else {
                    conflict("assessment already committed with different content")
                };
            }
            if r.status != ReviewStatus::Running {
                return conflict("review is not running");
            }
            current(c, &r)?;
            let a = Assessment {
                id: r.id.clone(),
                review_id: r.id.clone(),
                thesis_id: r.thesis.thesis_id.clone(),
                thesis_revision: r.thesis.revision,
                previous_assessment_id: r.previous_assessment_id.clone(),
                evidence_ids: r.evidence.iter().map(|e| e.id.clone()).collect(),
                attempt: attempt.number,
                runtime_identity: r
                    .runtime_identity
                    .clone()
                    .ok_or_else(|| ReviewError::Invalid("runtime identity missing".into()))?,
                created_at: now,
                draft,
            };
            c.execute(
                "INSERT INTO assessments(id,review_id,thesis_id,payload) VALUES (?1,?2,?3,?4)",
                params![a.id, a.review_id, a.thesis_id, serde_json::to_string(&a)?],
            )?;
            r.status = ReviewStatus::Completed;
            r.updated_at = now;
            r.detail = None;
            save_review(c, &r)?;
            Ok(a)
        })
    }
    fn finish(
        &self,
        attempt: &Attempt,
        status: ReviewStatus,
        detail: &str,
        now: DateTime<Utc>,
    ) -> ReviewResult<Review> {
        if !matches!(
            status,
            ReviewStatus::Interrupted | ReviewStatus::Failed | ReviewStatus::Blocked
        ) {
            return invalid("finish requires interrupted, failed or blocked");
        }
        text(detail, "finish reason", 4096)?;
        self.write(|c| {
            let mut r = review(c, &attempt.review_id)?;
            owns(&r, attempt)?;
            if r.status == ReviewStatus::Completed {
                return Ok(r);
            }
            if r.status != ReviewStatus::Running {
                return conflict("review is not running");
            }
            r.status = status;
            r.detail = Some(detail.into());
            r.updated_at = now;
            save_review(c, &r)?;
            Ok(r)
        })
    }
    fn recover(&self, now: DateTime<Utc>) -> ReviewResult<usize> {
        self.write(|c| {
            let mut q = c.prepare("SELECT payload FROM reviews WHERE status='running'")?;
            let records: Vec<String> = q
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            for payload in &records {
                let mut r: Review = serde_json::from_str(payload)?;
                r.status = ReviewStatus::Interrupted;
                r.updated_at = now;
                r.detail = Some("interrupted during previous application session".into());
                save_review(c, &r)?;
            }
            Ok(records.len())
        })
    }
    fn assessment(&self, id: &str) -> ReviewResult<Assessment> {
        self.read(|c| assessment(c, id))
    }
    fn assessments(&self, thesis_id: &str) -> ReviewResult<Vec<Assessment>> {
        self.read(|c| {
            let mut q =
                c.prepare("SELECT payload FROM assessments WHERE thesis_id=?1 ORDER BY sequence")?;
            let records: Vec<String> = q
                .query_map([thesis_id], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            records
                .iter()
                .map(|s| Ok(serde_json::from_str(s)?))
                .collect()
        })
    }
}
