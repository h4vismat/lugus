use lugus_financial::{
    application::{ingest, retrieve_document},
    capabilities::Provider,
    domain::*,
    plugin::*,
    storage::{Repository, SqliteRepository},
};
use serde_json::json;
use std::path::PathBuf;

#[tokio::test]
async fn python_process_to_sqlite_refresh_and_offline_reopen() {
    let temporary = tempfile::tempdir().unwrap();
    let database = temporary.path().join("evidence.sqlite");
    let mut repository = SqliteRepository::open(&database).unwrap();
    let mut plugin = Plugin::start(
        Manifest {
            id: "fixture".into(),
            version: "1".into(),
            protocol_version: 1,
            command: "python3".into(),
            args: vec!["plugin.py".into()],
        },
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        "local".into(),
        json!({"mode":"populated"}),
        Limits::default(),
    )
    .await
    .unwrap();
    let identity = plugin.identity().clone();
    let query = Query {
        company: CompanyId {
            namespace: "sec:cik".into(),
            value: "0000320193".into(),
        },
        filed_from: "2023-01-01".parse().unwrap(),
        filed_to: "2024-12-31".parse().unwrap(),
        forms: vec![],
        cursor: None,
        page_size: 1,
    };
    ingest(&mut repository, &mut plugin, &query).await.unwrap();
    ingest(&mut repository, &mut plugin, &query).await.unwrap();
    let checksum = retrieve_document(
        &mut repository,
        &mut plugin,
        "https://example.com/filing",
        100,
    )
    .await
    .unwrap();
    plugin.close().await.unwrap();
    drop(repository);
    let repository = SqliteRepository::open(database).unwrap();
    let snapshot = repository.snapshot(&identity, &query).unwrap();
    assert_eq!(snapshot.runs.len(), 2);
    assert!(snapshot.is_complete());
    assert_eq!(snapshot.filings.len(), 1);
    assert_eq!(snapshot.facts.len(), 2);
    assert_eq!(
        snapshot.facts[0].fact.value.as_str(),
        "12345678901234567890.001"
    );
    assert_eq!(snapshot.facts[0].metric.as_ref().unwrap().id, "assets");
    assert!(snapshot.facts[1].metric.is_none());
    assert_eq!(
        repository.stored_document(&checksum).unwrap(),
        b"<html>filing</html>"
    );
}

#[tokio::test]
async fn real_sec_plugin_initializes_without_network() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/sec-edgar/plugin.json");
    let (manifest, directory) = Manifest::load(path).unwrap();
    let mut plugin = Plugin::start(
        manifest,
        directory,
        "sec-test".into(),
        json!({"user_agent":"Lugus Test contact@example.com"}),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(plugin.capabilities().get("fundamentals"), Some(&1));
    assert_eq!(plugin.capabilities().get("filings"), Some(&1));
    assert_eq!(plugin.capabilities().get("company_resolution"), Some(&1));
    assert_eq!(plugin.identity().plugin_version, "0.2.0");
    plugin.close().await.unwrap();
}
