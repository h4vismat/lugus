use lugus_financial::{capabilities::*, domain::*, error::ErrorKind, market_data::*, plugin::*};
use serde_json::json;
use std::path::PathBuf;
async fn plugin(mode: &str) -> Plugin {
    Plugin::start(
        Manifest {
            id: "market-fixture".into(),
            version: "1".into(),
            protocol_version: 1,
            command: "python3".into(),
            args: vec!["market_plugin.py".into()],
        },
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        "test".into(),
        json!({"mode":mode}),
        Limits::default(),
    )
    .await
    .unwrap()
}
fn query() -> PriceQuery {
    PriceQuery {
        instrument: InstrumentId {
            namespace: "yahoo:symbol".into(),
            value: "AAPL".into(),
        },
        start: "2024-01-01".parse().unwrap(),
        end: "2024-01-31".parse().unwrap(),
        cursor: None,
        page_size: 100,
    }
}
#[tokio::test]
async fn independent_market_capability_pages_without_filings_capability() {
    let mut p = plugin("ok").await;
    let mut q = query();
    q.page_size = 1;
    let first = p.fetch_prices(&q).await.unwrap();
    assert_eq!(first.items.len(), 1);
    q.cursor = first.next_cursor;
    let second = p.fetch_prices(&q).await.unwrap();
    assert_eq!(second.items[0].date.to_string(), "2024-01-03");
    assert!(second.next_cursor.is_none());
    assert_eq!(first.coverage, second.coverage);
    let filings = Query {
        company: CompanyId {
            namespace: "sec:cik".into(),
            value: "1".into(),
        },
        filed_from: q.start,
        filed_to: q.end,
        forms: vec![],
        cursor: None,
        page_size: 1,
    };
    assert_eq!(
        p.list_filings(&filings).await.unwrap_err().kind,
        ErrorKind::Unsupported
    );
    p.close().await.unwrap();
}
#[tokio::test]
async fn invalid_market_results_close_connection() {
    for mode in ["wrong_instrument", "wrong_ohlc", "wrong_order"] {
        let mut p = plugin(mode).await;
        assert_eq!(
            p.fetch_prices(&query()).await.unwrap_err().kind,
            ErrorKind::Protocol
        );
        assert!(!p.is_running());
    }
}
#[tokio::test]
async fn unsupported_and_rate_limited_remain_explicit() {
    for (mode, kind) in [
        ("unsupported", ErrorKind::Unsupported),
        ("rate_limited", ErrorKind::RateLimited),
    ] {
        let mut p = plugin(mode).await;
        assert_eq!(p.fetch_prices(&query()).await.unwrap_err().kind, kind);
        p.close().await.unwrap();
    }
}
