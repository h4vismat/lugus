use lugus_financial::{
    application::market::ingest_prices,
    capabilities::Provider,
    market_data::*,
    plugin::*,
    storage::{SqliteRepository, market::MarketRepository},
};
use serde_json::json;
use std::path::PathBuf;

#[tokio::test]
async fn daily_history_process_paginates_persists_and_reopens_offline() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("history.sqlite");
    let mut repo = SqliteRepository::open(&db).unwrap();
    let mut plugin = Plugin::start(
        Manifest {
            id: "market-fixture".into(),
            version: "1".into(),
            protocol_version: 1,
            command: "python3".into(),
            args: vec!["market_plugin.py".into()],
        },
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        "local".into(),
        json!({}),
        Limits::default(),
    )
    .await
    .unwrap();
    let identity = plugin.identity().clone();
    let query = PriceQuery {
        instrument: InstrumentId {
            namespace: "yahoo:symbol".into(),
            value: "AAPL".into(),
        },
        start: "2024-01-01".parse().unwrap(),
        end: "2024-01-31".parse().unwrap(),
        cursor: None,
        page_size: 1,
    };
    ingest_prices(&mut repo, &mut plugin, &query).await.unwrap();
    ingest_prices(&mut repo, &mut plugin, &query).await.unwrap();
    plugin.close().await.unwrap();
    drop(repo);
    let repo = SqliteRepository::open(db).unwrap();
    let snapshot = repo.market_snapshot(&identity, &query).unwrap();
    assert_eq!(snapshot.prices.len(), 2);
    assert_eq!(snapshot.runs.len(), 2);
    assert_eq!(snapshot.prices[0].bar.open.as_str(), "100");
    assert_eq!(
        snapshot.prices[0]
            .bar
            .adjusted_close
            .as_ref()
            .unwrap()
            .as_str(),
        "100.5"
    );
    assert_eq!(
        snapshot.runs[1].coverage.as_ref().unwrap().completeness,
        Completeness::Unverified
    );
    assert_eq!(
        snapshot.runs[1].status,
        lugus_financial::storage::RunStatus::Complete
    );
    assert_eq!(
        repo.market_run_observations(snapshot.runs[1].id)
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn real_yfinance_protocol_initializes_without_library_or_network() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins/yfinance");
    // Initialization does not import the optional yfinance dependency.
    let mut plugin = Plugin::start(
        Manifest {
            id: "yfinance".into(),
            version: "0.2.0".into(),
            protocol_version: 1,
            command: "python3".into(),
            args: vec!["main.py".into()],
        },
        directory,
        "test".into(),
        json!({}),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(plugin.capabilities().get("market_data"), Some(&1));
    assert_eq!(plugin.capabilities().get("instrument_lookup"), Some(&1));
    assert_eq!(plugin.capabilities().len(), 2);
    plugin.close().await.unwrap();
}
