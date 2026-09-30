mod comparison_support;
use comparison_support::*;
use desktop_host::Bridge;
use serde_json::json;
#[tokio::test]
async fn native_comparison_has_no_runtime_requirement_and_reopens_offline() {
    let root = tempfile::tempdir().unwrap();
    let b = setup(root.path()).await;
    let c = call(
        &b,
        json!({"op":"create","request":"c","title":"Comparison"}),
    )
    .await;
    let other = call(&b, json!({"op":"create","request":"other","title":"Other"})).await;
    let job = comparison(
        &b,
        json!({"kind":"start","conversation":c["id"],"request":request("r")}),
    )
    .await;
    let done = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let s = comparison(
                &b,
                json!({"kind":"status","conversation":c["id"],"job":job["id"]}),
            )
            .await;
            if s["state"] != "running" {
                break s;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(done["state"], "complete", "{done}");
    let id = done["comparison_id"].clone();
    let source = comparison(
        &b,
        json!({"kind":"sources","conversation":c["id"],"id":id,"offset":0}),
    )
    .await;
    assert!(source["items"][0]["input"]["observation_id"].is_string());
    assert!(source["items"][0]["fact"]["value"].is_string());
    assert!(b.dispatch(&json!({"op":"comparison","command":{"kind":"read","conversation":other["id"],"id":id}}).to_string()).await.is_err());
    assert!(
        b.dispatch(r#"{"op":"comparison","command":{"kind":"providers","forged":true}}"#)
            .await
            .is_err()
    );
    let saved = comparison(&b, json!({"kind":"read","conversation":c["id"],"id":id})).await;
    b.shutdown().await.unwrap();
    drop(b);
    std::fs::remove_file(root.path().join("manifest.json")).unwrap();
    let b = Bridge::open(&root.path().join("desktop.json"), true)
        .await
        .unwrap();
    assert_eq!(
        comparison(&b, json!({"kind":"read","conversation":c["id"],"id":id})).await,
        saved
    );
    b.shutdown().await.unwrap();
}
