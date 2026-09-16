use desktop_host::Bridge;
use serde_json::{Value, json};
async fn call(b: &Bridge, c: Value) -> Value {
    b.dispatch(&json!({"op":"portfolio","command":c}).to_string())
        .await
        .unwrap()
}
#[tokio::test]
async fn native_history_is_scoped_paged_and_usable_offline() {
    let dir = tempfile::tempdir().unwrap();
    let limits = lugus_app::Limits {
        max_output_bytes: 32768,
        max_read_page_bytes: 32768,
        ..Default::default()
    };
    std::fs::write(
        dir.path().join("app.json"),
        json!({"financial_path":"fin","application_path":"app","providers":[],"limits":limits})
            .to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("desktop.json"),
        json!({"application_config":"app.json"}).to_string(),
    )
    .unwrap();
    let b = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    let p=call(&b,json!({"kind":"execute","request":{"request_id":"p","portfolio_id":null,"expected_revision":"0","mutation":{"kind":"create_portfolio","name":"P"}}})).await;
    call(&b,json!({"kind":"execute","request":{"request_id":"a","portfolio_id":p["portfolio_id"],"expected_revision":"1","mutation":{"kind":"create_account","name":"Cash","start":"2026-01-01","opening":{"kind":"existing","cash":"100","lots":[]},"events":[]}}})).await;
    let mut r=call(&b,json!({"kind":"history_start","request":{"request_id":"h","portfolio_id":p["portfolio_id"],"account_id":null,"expected_revision":"2","range":{"start":"2026-01-02","end":"2026-09-10"},"refresh":"missing"}})).await;
    r = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while r["status"] == "running" {
            tokio::task::yield_now().await;
            r = call(
                &b,
                json!({"kind":"history_status","portfolio_id":p["portfolio_id"],"id":r["id"]}),
            )
            .await;
        }
        r
    })
    .await
    .unwrap();
    assert_eq!(r["status"], "partial", "{r}");
    assert_eq!(r["summary"]["portfolio_return_percent"], "0");
    assert!(r.get("evidence").is_none());
    let mut offset = 0;
    let mut dates = std::collections::BTreeSet::new();
    loop {
        let page=call(&b,json!({"kind":"history_read","portfolio_id":p["portfolio_id"],"id":r["id"],"offset":offset})).await;
        assert!(page["items"].as_array().unwrap().len() <= 200);
        if offset == 0 { assert!(serde_json::to_vec(&page).unwrap().len() > 32768); }
        assert_eq!(page["key"]["revision"], "2");
        for row in page["items"].as_array().unwrap() {
            assert!(dates.insert(row["date"].as_str().unwrap().to_owned()));
        }
        match page["next_offset"].as_u64() {
            Some(n) => {
                assert!(n > offset);
                offset = n;
            }
            None => break,
        }
    }
    assert!(dates.len() > 200);
    assert_eq!(dates.len() as u64, r["row_count"].as_u64().unwrap());
    assert!(b.dispatch(&json!({"op":"portfolio","command":{"kind":"history_read","portfolio_id":"wrong","id":r["id"],"offset":0}}).to_string()).await.is_err());
    b.shutdown().await.unwrap();
    let b = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    let cached=call(&b,json!({"kind":"history_latest","portfolio_id":p["portfolio_id"],"account_id":null,"range":{"start":"2026-01-02","end":"2026-09-10"}})).await;
    assert_eq!(cached["id"], r["id"]);
    b.shutdown().await.unwrap();
}
#[tokio::test]
async fn data_providers_are_available_without_a_chat_agent_profile() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lugus-app/tests/fixtures/worker.py")
        .canonicalize()
        .unwrap();
    std::fs::write(dir.path().join("plugin.json"),json!({"id":"yfinance","version":"0.3.0","protocol_version":1,"command":"python3","args":[fixture]}).to_string()).unwrap();
    std::fs::write(dir.path().join("app.json"),json!({"financial_path":"fin","application_path":"app","providers":[{"instance_id":"p","manifest":"plugin.json","active":true,"config":{"mode":"history_ok","barrier":dir.path().join("barrier"),"plugin_id":"yfinance","version":"0.3.0"}}]}).to_string()).unwrap();
    std::fs::write(
        dir.path().join("desktop.json"),
        json!({"application_config":"app.json"}).to_string(),
    )
    .unwrap();
    let b = Bridge::open(&dir.path().join("desktop.json"), false)
        .await
        .unwrap();
    let info = b.dispatch("{\"op\":\"info\"}").await.unwrap();
    assert_eq!(info["runtime_available"], false);
    assert_eq!(info["offline"], false);
    assert_eq!(
        call(&b, json!({"kind":"history_providers"}))
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    b.shutdown().await.unwrap();
}
