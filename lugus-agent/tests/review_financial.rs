use lugus_agent::reviews::*;
use lugus_financial::{
    domain::*,
    storage::{Repository, SqliteRepository},
};
use serde_json::json;

fn query() -> Query {
    serde_json::from_value(json!({"company":{"namespace":"sec:cik","value":"1"},"filed_from":"2024-01-01","filed_to":"2024-12-31","forms":[],"page_size":100})).unwrap()
}
fn provider() -> ProviderIdentity {
    ProviderIdentity {
        instance_id: "local".into(),
        plugin_id: "fixture".into(),
        plugin_version: "1".into(),
    }
}
fn fact(value: &str) -> Fact {
    serde_json::from_value(json!({"company":query().company,"namespace":"us-gaap","concept":"Assets","value":value,"unit":"USD","period":{"kind":"instant","date":"2023-12-31"},"filing_id":"a","form":"10-K","filed":"2024-02-01","source_url":"https://example.test/a","retrieved_at":"2026-09-10T12:00:00Z"})).unwrap()
}
#[test]
fn capture_retains_source_revisions_and_partial_ingestion_status() {
    let dir = tempfile::tempdir().unwrap();
    let mut financial = SqliteRepository::open(dir.path().join("financial.db")).unwrap();
    let run = financial.start_run(&provider(), &query(), "facts").unwrap();
    financial
        .save_facts_page(
            run,
            &Page {
                items: vec![fact("100")],
                next_cursor: None,
            },
            &map_metric,
        )
        .unwrap();
    financial.finish_run(run, None).unwrap();
    let first = capture_financial_evidence(&financial, &provider(), &query()).unwrap();
    let old = first.iter().find(|e| e.content["kind"] == "fact").unwrap();
    assert_eq!(old.content["observation"]["fact"]["value"], "100");
    assert_eq!(old.content["provider"]["instance_id"], "local");
    assert_eq!(old.content["observation"]["observation_id"], 1);
    let store = SqliteReviewStore::open(dir.path().join("agent.db")).unwrap();
    let now = "2026-09-10T12:00:00Z".parse().unwrap();
    store.save_thesis("t", 0, "Thesis", now).unwrap();
    store.enqueue("a", "t", 1, first.clone(), now).unwrap();
    let next = financial.start_run(&provider(), &query(), "facts").unwrap();
    financial
        .save_facts_page(
            next,
            &Page {
                items: vec![fact("80")],
                next_cursor: Some("more".into()),
            },
            &map_metric,
        )
        .unwrap();
    let second = capture_financial_evidence(&financial, &provider(), &query()).unwrap();
    assert!(
        second
            .iter()
            .any(|e| e.content["observation"]["fact"]["value"] == "80")
    );
    assert!(second.iter().any(|e| e.id == old.id));
    let scope = second
        .iter()
        .find(|e| e.content["kind"] == "snapshot_scope")
        .unwrap();
    assert_eq!(scope.content["all_runs_complete"], false);
    assert_eq!(scope.content["runs"][1]["status"], "running");
    drop(financial);
    assert_eq!(store.review("a").unwrap().evidence, first);
    assert_eq!(
        store
            .review("a")
            .unwrap()
            .evidence
            .iter()
            .find(|e| e.id == old.id)
            .unwrap()
            .content["observation"]["fact"]["value"],
        "100"
    );
}
#[test]
fn empty_snapshot_is_explicitly_incomplete_and_invalid_query_fails() {
    let repo = SqliteRepository::open(":memory:").unwrap();
    let items = capture_financial_evidence(&repo, &provider(), &query()).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].content["all_runs_complete"], false);
    let mut q = query();
    q.page_size = 0;
    assert!(capture_financial_evidence(&repo, &provider(), &q).is_err());
}

#[test]
fn large_snapshots_can_be_enumerated_before_explicit_bounded_selection() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let run = repo.start_run(&provider(), &query(), "facts").unwrap();
    let facts = (0..65).map(|i| fact(&i.to_string())).collect();
    repo.save_facts_page(
        run,
        &Page {
            items: facts,
            next_cursor: None,
        },
        &map_metric,
    )
    .unwrap();
    repo.finish_run(run, None).unwrap();
    let mut items = capture_financial_evidence(&repo, &provider(), &query()).unwrap();
    assert_eq!(items.len(), 66);
    let store = SqliteReviewStore::open(":memory:").unwrap();
    let now = "2026-09-10T12:00:00Z".parse().unwrap();
    store.save_thesis("t", 0, "Thesis", now).unwrap();
    assert!(matches!(
        store.enqueue("large", "t", 1, items.clone(), now),
        Err(ReviewError::Invalid(_))
    ));
    items.truncate(2);
    store.enqueue("selected", "t", 1, items, now).unwrap();
}
