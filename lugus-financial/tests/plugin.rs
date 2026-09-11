use lugus_financial::{capabilities::*, domain::*, error::ErrorKind, plugin::*};
use serde_json::json;
use std::{path::PathBuf, time::Duration};

async fn start(mode: &str) -> lugus_financial::error::Result<Plugin> {
    let manifest = Manifest {
        id: "fixture".into(),
        version: "1".into(),
        protocol_version: 1,
        command: "python3".into(),
        args: vec!["plugin.py".into()],
    };
    Plugin::start(
        manifest,
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        "test".into(),
        json!({"mode":mode}),
        Limits {
            timeout: Duration::from_millis(500),
            max_response_bytes: 2048,
        },
    )
    .await
}
fn query() -> Query {
    Query {
        company: CompanyId {
            namespace: "sec:cik".into(),
            value: "0000320193".into(),
        },
        filed_from: "2023-01-01".parse().unwrap(),
        filed_to: "2024-12-31".parse().unwrap(),
        forms: vec![],
        cursor: None,
        page_size: 10,
    }
}
#[tokio::test]
async fn process_negotiates_and_drains_stderr_without_deadlock() {
    let mut plugin = start("stderr").await.unwrap();
    assert_eq!(plugin.identity().plugin_id, "fixture");
    assert!(
        plugin
            .list_filings(&query())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    plugin.close().await.unwrap();
}
#[tokio::test]
async fn host_rejects_incompatible_version_and_unadvertised_capability() {
    assert_eq!(
        start("wrong_version").await.err().unwrap().kind,
        ErrorKind::Protocol
    );
    let mut plugin = start("unsupported").await.unwrap();
    assert_eq!(
        plugin.fetch_facts(&query()).await.unwrap_err().kind,
        ErrorKind::Unsupported
    );
    plugin.close().await.unwrap();
}
#[tokio::test]
async fn protocol_failures_and_timeouts_close_the_child() {
    for (mode, kind) in [
        ("eof", ErrorKind::Protocol),
        ("timeout", ErrorKind::Timeout),
        ("malformed", ErrorKind::Protocol),
        ("oversized", ErrorKind::Protocol),
    ] {
        let mut plugin = start(mode).await.unwrap();
        assert_eq!(plugin.list_filings(&query()).await.unwrap_err().kind, kind);
        assert!(!plugin.is_running());
    }
}

#[tokio::test]
async fn framed_source_failures_allow_a_later_request_on_the_same_process() {
    for (mode, kind) in [
        ("source_timeout", ErrorKind::Timeout),
        ("source_unavailable", ErrorKind::Unavailable),
    ] {
        let mut plugin = start(mode).await.unwrap();
        assert_eq!(plugin.fetch_facts(&query()).await.unwrap_err().kind, kind);
        assert!(
            plugin.is_running(),
            "a completed source error must not poison the connection"
        );
        assert!(plugin.fetch_facts(&query()).await.unwrap().items.is_empty());
        plugin.close().await.unwrap();
    }
}
#[tokio::test]
async fn rate_limit_is_distinct_from_empty_success() {
    let mut plugin = start("error").await.unwrap();
    let error = plugin.fetch_facts(&query()).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::RateLimited);
    assert_eq!(error.retry_after_seconds, Some(3));
    assert!(plugin.is_running());
    plugin.close().await.unwrap();
}

#[tokio::test]
async fn cancelling_a_request_invalidates_the_connection() {
    let mut plugin = start("timeout").await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), plugin.list_filings(&query()))
            .await
            .is_err()
    );
    assert!(
        !plugin.is_running(),
        "cancelled request must not leave a reusable connection"
    );
    assert_eq!(
        plugin.list_filings(&query()).await.unwrap_err().kind,
        ErrorKind::Unavailable
    );
    plugin.close().await.unwrap();
}
