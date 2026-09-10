use lugus_agent::reviews::*;
use serde_json::json;

fn now() -> chrono::DateTime<chrono::Utc> {
    "2026-09-10T12:00:00Z".parse().unwrap()
}
fn evidence() -> Evidence {
    Evidence::new(
        "financial:provider:fact:1",
        "Assets",
        json!({"value":"100", "source_url":"https://example.test/filing"}),
    )
    .unwrap()
}
fn draft(id: &str) -> AssessmentDraft {
    AssessmentDraft {
        interpretation: "Assets may support resilience".into(),
        conclusion: "Insufficient evidence of resilience".into(),
        supporting: vec![EvidenceClaim {
            text: "Reported assets are 100".into(),
            evidence_ids: vec![id.into()],
        }],
        opposing: vec![],
        uncertainty: vec!["Liabilities unavailable".into()],
        open_questions: vec!["How large are liabilities?".into()],
        changes: "First assessment".into(),
    }
}
fn setup() -> (tempfile::TempDir, SqliteReviewStore, Evidence) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteReviewStore::open(dir.path().join("agent.db")).unwrap();
    store
        .save_thesis("t", 0, "Assets imply resilience", now())
        .unwrap();
    let item = evidence();
    store
        .enqueue("r", "t", 1, vec![item.clone()], now())
        .unwrap();
    (dir, store, item)
}
#[test]
fn reopened_history_preserves_thesis_and_exact_evidence() {
    let (dir, store, item) = setup();
    let attempt = store.claim("r", "fake:v1", now()).unwrap();
    store.submit(&attempt, draft(&item.id), now()).unwrap();
    drop(store);
    let store = SqliteReviewStore::open(dir.path().join("agent.db")).unwrap();
    assert_eq!(store.review("r").unwrap().status, ReviewStatus::Completed);
    assert_eq!(
        store.review("r").unwrap().evidence[0].content["value"],
        "100"
    );
    store.save_thesis("t", 1, "Revised thesis", now()).unwrap();
    assert_eq!(
        store.thesis("t", Some(1)).unwrap().text,
        "Assets imply resilience"
    );
    assert_eq!(store.thesis("t", None).unwrap().revision, 2);
    assert_eq!(
        store.assessments("t").unwrap()[0].draft.open_questions,
        ["How large are liabilities?"]
    );
}
#[test]
fn repeated_requests_and_submissions_are_idempotent_but_conflicting_content_is_rejected() {
    let (_dir, store, item) = setup();
    store
        .enqueue("r", "t", 1, vec![item.clone()], now())
        .unwrap();
    assert!(matches!(
        store.enqueue("r", "t", 1, vec![], now()),
        Err(ReviewError::Conflict(_))
    ));
    let attempt = store.claim("r", "fake", now()).unwrap();
    let d = draft(&item.id);
    let first = store.submit(&attempt, d.clone(), now()).unwrap();
    assert_eq!(store.submit(&attempt, d, now()).unwrap().id, first.id);
    let mut changed = draft(&item.id);
    changed.conclusion = "Different".into();
    assert!(matches!(
        store.submit(&attempt, changed, now()),
        Err(ReviewError::Conflict(_))
    ));
    store
        .finish(&attempt, ReviewStatus::Failed, "transport failed", now())
        .unwrap();
    assert_eq!(store.recover(now()).unwrap(), 0);
    assert_eq!(store.assessments("t").unwrap().len(), 1);
    assert_eq!(store.review("r").unwrap().status, ReviewStatus::Completed);
}
#[test]
fn invalid_references_and_stale_thesis_cannot_commit() {
    let (_dir, store, item) = setup();
    let attempt = store.claim("r", "fake", now()).unwrap();
    assert!(matches!(
        store.submit(&attempt, draft("unknown"), now()),
        Err(ReviewError::Invalid(_))
    ));
    assert_eq!(store.assessments("t").unwrap().len(), 0);
    store.save_thesis("t", 1, "Changed", now()).unwrap();
    assert!(matches!(
        store.save_thesis("t", 1, "Overwrite", now()),
        Err(ReviewError::Conflict(_))
    ));
    assert!(matches!(
        store.submit(&attempt, draft(&item.id), now()),
        Err(ReviewError::Conflict(_))
    ));
    assert_eq!(store.review("r").unwrap().status, ReviewStatus::Running);
}
#[test]
fn recovery_fences_old_attempts_and_open_does_not_interrupt_active_work() {
    let (dir, store, item) = setup();
    let old = store.claim("r", "fake", now()).unwrap();
    let other = SqliteReviewStore::open(dir.path().join("agent.db")).unwrap();
    assert_eq!(other.review("r").unwrap().status, ReviewStatus::Running);
    other
        .enqueue("r2", "t", 1, vec![item.clone()], now())
        .unwrap();
    assert!(matches!(
        other.claim("r2", "fake", now()),
        Err(ReviewError::Conflict(_))
    ));
    assert_eq!(other.recover(now()).unwrap(), 1);
    let new = other.claim("r", "fake", now()).unwrap();
    assert_eq!(new.number, old.number + 1);
    assert!(matches!(
        store.submit(&old, draft(&item.id), now()),
        Err(ReviewError::Conflict(_))
    ));
    assert!(matches!(
        store.finish(&old, ReviewStatus::Failed, "late", now()),
        Err(ReviewError::Conflict(_))
    ));
    other.submit(&new, draft(&item.id), now()).unwrap();
    assert!(matches!(
        store.claim("r2", "fake", now()),
        Err(ReviewError::Conflict(_))
    ));
}
#[test]
fn a_new_review_keeps_previous_assessment_and_selected_inputs() {
    let (_dir, store, item) = setup();
    let first = store.claim("r", "fake", now()).unwrap();
    let saved = store.submit(&first, draft(&item.id), now()).unwrap();
    let next = Evidence::new("financial:provider:fact:2", "Assets", json!({"value":"80"})).unwrap();
    store
        .enqueue("r2", "t", 1, vec![next.clone()], now())
        .unwrap();
    let r = store.review("r2").unwrap();
    assert_eq!(r.previous_assessment_id, Some(saved.id));
    assert_eq!(r.evidence[0].content["value"], "80");
    assert_eq!(
        store.review("r").unwrap().evidence[0].content["value"],
        "100"
    );
}
#[test]
fn malformed_and_oversized_inputs_are_rejected_without_writes() {
    let (_dir, store, mut item) = setup();
    item.content = json!({"value":"forged"});
    assert!(matches!(
        store.enqueue("x", "t", 1, vec![item], now()),
        Err(ReviewError::Invalid(_))
    ));
    assert!(
        store
            .save_thesis("t", 1, &"x".repeat(16385), now())
            .is_err()
    );
    assert!(store.save_thesis("", 0, "thesis", now()).is_err());
    let a = store.claim("r", "fake", now()).unwrap();
    let mut d = draft(&evidence().id);
    d.supporting[0].evidence_ids.clear();
    assert!(store.submit(&a, d, now()).is_err());
    assert_eq!(store.thesis("t", None).unwrap().revision, 1);
}

#[test]
fn failed_completion_transaction_rolls_back_the_assessment_insert() {
    let (dir, store, item) = setup();
    let attempt = store.claim("r", "fake", now()).unwrap();
    let connection = rusqlite::Connection::open(dir.path().join("agent.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_completion BEFORE UPDATE ON reviews WHEN NEW.status='completed' BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    assert!(matches!(
        store.submit(&attempt, draft(&item.id), now()),
        Err(ReviewError::Storage(_))
    ));
    assert!(store.assessments("t").unwrap().is_empty());
    assert_eq!(store.review("r").unwrap().status, ReviewStatus::Running);
    connection
        .execute_batch("DROP TRIGGER fail_completion;")
        .unwrap();
    store.submit(&attempt, draft(&item.id), now()).unwrap();
    assert_eq!(store.assessments("t").unwrap().len(), 1);
}
#[test]
fn concurrent_identical_submissions_commit_only_one_assessment() {
    let (dir, store, item) = setup();
    let attempt = store.claim("r", "fake", now()).unwrap();
    let other = SqliteReviewStore::open(dir.path().join("agent.db")).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let gate = barrier.clone();
    let a = attempt.clone();
    let d = draft(&item.id);
    let worker = std::thread::spawn(move || {
        gate.wait();
        other.submit(&a, d, now()).unwrap()
    });
    barrier.wait();
    let first = store.submit(&attempt, draft(&item.id), now()).unwrap();
    assert_eq!(worker.join().unwrap(), first);
    assert_eq!(store.assessments("t").unwrap().len(), 1);
}
#[test]
fn schema_guards_do_not_modify_unrelated_or_future_databases() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("other.db");
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch("CREATE TABLE unrelated(value TEXT);")
        .unwrap();
    assert!(matches!(
        SqliteReviewStore::open(&db),
        Err(ReviewError::Invalid(_))
    ));
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    c.execute_batch("PRAGMA application_id=1280651858; PRAGMA user_version=2;")
        .unwrap();
    assert!(matches!(
        SqliteReviewStore::open(&db),
        Err(ReviewError::Invalid(_))
    ));
}
