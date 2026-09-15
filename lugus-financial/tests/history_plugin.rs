use lugus_financial::{
    capabilities::HistoricalPricesProvider,
    error::ErrorKind,
    historical_prices::HistoryQuery,
    plugin::{Limits, Manifest, Plugin},
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
async fn history_pages_preserve_calendar_and_scope() {
    let mut p = plugin("ok").await;
    let mut q = query();
    let mut dates = vec![];
    let mut manifest = None;
    loop {
        let page = p.fetch_history(&q).await.unwrap();
        if let Some(m) = &manifest {
            assert_eq!(m, &page.manifest);
        } else {
            manifest = Some(page.manifest);
        }
        dates.extend(page.items.iter().map(|d| d.date.to_string()));
        q.cursor = page.next_cursor;
        if q.cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        dates,
        vec![
            "2025-12-31",
            "2026-01-01",
            "2026-01-02",
            "2026-01-03",
            "2026-01-04",
            "2026-01-05"
        ]
    );
    p.close().await.unwrap();
}
#[tokio::test]
async fn malformed_history_closes_plugin_and_source_errors_remain_typed() {
    for mode in [
        "wrong_instrument",
        "wrong_anchor",
        "wrong_order",
        "invalid_split",
    ] {
        let mut p = plugin(mode).await;
        assert_eq!(
            p.fetch_history(&query()).await.unwrap_err().kind,
            ErrorKind::Protocol
        );
        assert!(!p.is_running());
    }
    for (mode, kind) in [
        ("unsupported", ErrorKind::Unsupported),
        ("rate_limited", ErrorKind::RateLimited),
    ] {
        let mut p = plugin(mode).await;
        assert_eq!(p.fetch_history(&query()).await.unwrap_err().kind, kind);
        p.close().await.unwrap();
    }
}
