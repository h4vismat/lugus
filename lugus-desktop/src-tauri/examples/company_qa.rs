//! Explicit synthetic company QA host: disposable databases, deterministic agent/provider.
#[path = "../tests/support/mod.rs"]
mod support;
use std::io::{self, BufRead, Write};

#[tokio::main]
async fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("disposable QA directory"));
    let (bridge, _) = support::setup_in(&root, support::Factory::default()).await;
    let bridge = bridge
        .with_company_store(&root.join("companies.sqlite"))
        .unwrap();
    for line in io::stdin().lock().lines() {
        let output = match bridge.dispatch(&line.expect("input")).await {
            Ok(value) => serde_json::json!({"value":value}),
            Err(error) => serde_json::json!({"error":error}),
        };
        println!("{output}");
        io::stdout().flush().expect("flush output");
    }
    bridge.shutdown().await.expect("shutdown");
}
