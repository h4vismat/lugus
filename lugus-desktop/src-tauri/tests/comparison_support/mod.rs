#![allow(dead_code)]
use desktop_host::Bridge;
use serde_json::{Value, json};
use std::path::Path;
pub async fn setup(root: &Path) -> Bridge {
    let worker = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lugus-app/tests/fixtures/worker.py")
        .canonicalize()
        .unwrap();
    std::fs::write(root.join("manifest.json"),json!({"id":"sec-edgar","version":"0.3.0","protocol_version":1,"command":"python3","args":[worker]}).to_string()).unwrap();
    std::fs::write(root.join("application.json"),json!({"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[{"instance_id":"sec","manifest":"manifest.json","active":true,"config":{"mode":"apple_comparison","plugin_id":"sec-edgar","version":"0.3.0","barrier":root.join("provider")}}]}).to_string()).unwrap();
    std::fs::write(
        root.join("desktop.json"),
        r#"{"application_config":"application.json"}"#,
    )
    .unwrap();
    Bridge::open(&root.join("desktop.json"), false)
        .await
        .unwrap()
}
pub fn request(id: &str) -> Value {
    json!({"request_id":id,"subjects":[{"text":"AAPL","exchange":null},{"text":"MSFT","exchange":null}],"period_end":"2024-12-31","years":3,"revenue_basis":"contract_revenue_excluding_tax"})
}
pub async fn call(b: &Bridge, c: Value) -> Value {
    b.dispatch(&c.to_string()).await.unwrap()
}
pub async fn comparison(b: &Bridge, c: Value) -> Value {
    call(b, json!({"op":"comparison","command":c})).await
}
