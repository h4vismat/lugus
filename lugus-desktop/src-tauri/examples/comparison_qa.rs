//! Disposable comparison QA host; synthetic financial evidence and no agent runtime.
#[path = "../tests/comparison_support/mod.rs"]
mod support;
use desktop_host::Bridge;
use std::io::{self, BufRead, Write};
#[tokio::main]
async fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("disposable QA directory"));
    let mut bridge = support::setup(&root).await;
    for line in io::stdin().lock().lines() {
        let line = line.expect("input");
        let command: serde_json::Value = serde_json::from_str(&line).expect("json");
        let result = if let Some(mode) = command.get("qa_mode").and_then(|v| v.as_str()) {
            match mode {
                "changed" | "partial" | "conflict" => {
                    std::fs::write(root.join("provider").join(mode), "").unwrap()
                }
                "offline" => {
                    bridge.shutdown().await.unwrap();
                    std::fs::remove_file(root.join("manifest.json")).unwrap();
                    bridge = Bridge::open(&root.join("desktop.json"), true)
                        .await
                        .unwrap();
                }
                _ => panic!("unknown QA mode"),
            }
            Ok(serde_json::json!({"ok":true}))
        } else {
            bridge.dispatch(&line).await
        };
        let output = match result {
            Ok(value) => serde_json::json!({"value":value}),
            Err(error) => serde_json::json!({"error":error}),
        };
        println!("{output}");
        io::stdout().flush().expect("flush");
    }
    bridge.shutdown().await.unwrap();
}
