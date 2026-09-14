use desktop_host::Bridge;
use serde_json::{Value, json};
async fn call(b: &Bridge, c: Value) -> Value {
    b.dispatch(&json!({"op":"portfolio","command":c}).to_string())
        .await
        .unwrap()
}
#[tokio::test]
async fn portfolio_pages_fit_small_response_budgets_without_skipping_items() {
    let dir = tempfile::tempdir().unwrap();
    let limits = lugus_app::Limits {
        max_output_bytes: 32768,
        max_read_page_bytes: 32768,
        ..Default::default()
    };
    std::fs::write(dir.path().join("app.json"), json!({"financial_path":"financial.sqlite","application_path":"app.sqlite","providers":[],"limits":limits}).to_string()).unwrap();
    std::fs::write(
        dir.path().join("desktop.json"),
        json!({"application_config":"app.json"}).to_string(),
    )
    .unwrap();
    let bridge = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    let mut ids = std::collections::BTreeSet::new();
    for i in 0..120 {
        let receipt = call(&bridge, json!({"kind":"execute","request":{"request_id":format!("p{i}"),"portfolio_id":null,"expected_revision":"0","mutation":{"kind":"create_portfolio","name":"P".repeat(256)}}})).await;
        ids.insert(receipt["portfolio_id"].as_str().unwrap().to_owned());
    }
    let mut offset = 0;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        let page = call(&bridge, json!({"kind":"list","offset":offset})).await;
        assert!(serde_json::to_vec(&page).unwrap().len() <= 32768);
        assert!(page["items"].as_array().unwrap().len() < 100);
        for item in page["items"].as_array().unwrap() {
            assert!(seen.insert(item["id"].as_str().unwrap().to_owned()));
        }
        match page["next_offset"].as_u64() {
            Some(next) => {
                assert!(next > offset);
                offset = next;
            }
            None => break,
        }
    }
    assert_eq!(seen, ids);
    bridge.shutdown().await.unwrap();
}
#[tokio::test]
async fn portfolio_entry_and_preview_work_without_an_agent() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.json"),
        json!({"financial_path":"financial.sqlite","application_path":"app.sqlite","providers":[]})
            .to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("desktop.json"),
        json!({"application_config":"app.json"}).to_string(),
    )
    .unwrap();
    let bridge = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    let p=call(&bridge,json!({"kind":"execute","request":{"request_id":"p","portfolio_id":null,"expected_revision":"0","mutation":{"kind":"create_portfolio","name":"Portfolio"}}})).await;
    let request = json!({"request_id":"a","portfolio_id":p["portfolio_id"],"expected_revision":"1","mutation":{"kind":"create_account","name":"Broker","start":"2026-01-01","opening":{"kind":"existing","cash":"123.45","lots":[]},"events":[]}});
    let preview = call(&bridge, json!({"kind":"preview","request":request})).await;
    assert_eq!(preview["view"]["valuation"]["cash"], "123.45");
    let before = call(
        &bridge,
        json!({"kind":"overview","portfolio_id":p["portfolio_id"],"account_id":null}),
    )
    .await;
    assert_eq!(before["valuation"]["cash"], "0");
    call(&bridge, json!({"kind":"execute","request":request})).await;
    let after = call(
        &bridge,
        json!({"kind":"overview","portfolio_id":p["portfolio_id"],"account_id":null}),
    )
    .await;
    assert_eq!(after["valuation"]["cash"], "123.45");
    assert_eq!(after["revision"], "2");
    bridge.shutdown().await.unwrap();
    let reopened = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    let saved = call(
        &reopened,
        json!({"kind":"overview","portfolio_id":p["portfolio_id"],"account_id":null}),
    )
    .await;
    assert_eq!(saved["valuation"]["cash"], "123.45");
    reopened.shutdown().await.unwrap();
}
