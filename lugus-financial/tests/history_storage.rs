use lugus_financial::{
    application::history::ingest_history,
    capabilities::Provider,
    historical_prices::HistoryQuery,
    plugin::{Limits, Manifest, Plugin},
    storage::{RunStatus, SqliteRepository, bounded::ReadLimits, history::HistoryRepository},
};
use serde_json::json;
async fn plugin(mode: &str) -> Plugin {
    Plugin::start(
        Manifest {
            id: "history-fixture".into(),
            version: "1".into(),
            protocol_version: 1,
            command: "python3".into(),
            args: vec!["history_plugin.py".into()],
        },
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        "p".into(),
        json!({"mode":mode}),
        Limits::default(),
    )
    .await
    .unwrap()
}
fn query() -> HistoryQuery {
    serde_json::from_value(json!({"instrument":{"namespace":"yahoo:symbol","value":"TEST"},"start":"2026-01-02","end":"2026-01-05","anchor":"2026-01-05","cursor":null,"page_size":2})).unwrap()
}
#[tokio::test]
async fn immutable_history_pages_survive_reopen_and_enforce_scope_and_budgets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("evidence.sqlite");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let mut p = plugin("ok").await;
    let provider = p.identity().clone();
    let id = ingest_history(&mut repo, &mut p, &query()).await.unwrap();
    drop(repo);
    let repo = SqliteRepository::open(&path).unwrap();
    let limits = ReadLimits {
        max_items: 2,
        max_bytes: 16000,
    };
    let first = repo.history_page(&provider, id, 0, limits).unwrap();
    assert_eq!(first.run.status, RunStatus::Complete);
    assert_eq!(first.items.len(), 2);
    assert_eq!(first.next_offset, Some(2));
    assert_eq!(first.items[1].day.close, None);
    assert!(
        repo.history_page(
            &provider,
            id,
            0,
            ReadLimits {
                max_items: 2,
                max_bytes: 10
            }
        )
        .is_err()
    );
    let mut wrong = provider.clone();
    wrong.instance_id = "other".into();
    assert!(repo.history_page(&wrong, id, 0, limits).is_err());
    let last = repo.history_page(&provider, id, 4, limits).unwrap();
    assert_eq!(last.next_offset, None);
    assert_eq!(last.items[1].day.date.to_string(), "2026-01-05");
    p.close().await.unwrap();
}
#[tokio::test]
async fn changing_manifest_cannot_publish_successful_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = SqliteRepository::open(dir.path().join("e.sqlite")).unwrap();
    let mut p = plugin("changed_manifest").await;
    let provider = p.identity().clone();
    assert!(ingest_history(&mut repo, &mut p, &query()).await.is_err());
    let run = repo
        .history_run(
            &provider,
            1,
            ReadLimits {
                max_items: 10,
                max_bytes: 16000,
            },
        )
        .unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    p.close().await.unwrap();
}
