use lugus_financial::{domain::CompanyId, error::ErrorKind, plugin::*, resolution::*};
use serde_json::json;
use std::path::PathBuf;
async fn start(mode: &str) -> Plugin {
    Plugin::start(
        Manifest {
            id: "resolution-fixture".into(),
            version: "1".into(),
            protocol_version: 1,
            command: "python3".into(),
            args: vec!["resolution_plugin.py".into()],
        },
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        "local".into(),
        json!({"mode":mode}),
        Limits::default(),
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn real_process_search_and_lookup_are_validated() {
    let mut plugin = start("ok").await;
    let request = SearchRequest {
        query: parse_input("$IBM").unwrap().primary,
        page_size: 10,
        cursor: None,
    };
    assert_eq!(
        plugin.search_companies(&request).await.unwrap().items[0]
            .identifier
            .value,
        "0000051143"
    );
    assert_eq!(
        plugin
            .lookup_company(&LookupRequest {
                identifier: CompanyId {
                    namespace: "sec:cik".into(),
                    value: "0000051143".into()
                }
            })
            .await
            .unwrap()
            .name,
        "IBM"
    );
    plugin.close().await.unwrap();
}
#[tokio::test]
async fn unadvertised_and_forged_resolution_results_are_rejected() {
    for (mode, expected) in [
        ("unsupported", ErrorKind::Unsupported),
        ("wrong_match", ErrorKind::Protocol),
    ] {
        let mut plugin = start(mode).await;
        let q = SearchRequest {
            query: parse_input("$IBM").unwrap().primary,
            page_size: 10,
            cursor: None,
        };
        assert_eq!(
            plugin.search_companies(&q).await.unwrap_err().kind,
            expected
        );
        plugin.close().await.unwrap();
    }
    let mut plugin = start("wrong_entity").await;
    assert_eq!(
        plugin
            .lookup_company(&LookupRequest {
                identifier: CompanyId {
                    namespace: "sec:cik".into(),
                    value: "0000051143".into()
                }
            })
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Protocol
    );
    assert!(!plugin.is_running());
    plugin.close().await.unwrap();
}
