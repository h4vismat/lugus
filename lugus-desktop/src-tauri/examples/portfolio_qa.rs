//! Native bridge for deterministic renderer QA with a disposable config.
use desktop_host::Bridge;
use std::io::{self, BufRead, Write};
#[tokio::main]
async fn main() {
    let config = std::env::args().nth(1).expect("desktop config path");
    let offline = !std::env::args().any(|arg| arg == "--online");
    let bridge = Bridge::open(std::path::Path::new(&config), offline)
        .await
        .expect("open disposable QA config");
    for line in io::stdin().lock().lines() {
        let line = line.expect("input");
        let output = match bridge.dispatch(&line).await {
            Ok(value) => serde_json::json!({"value":value}),
            Err(error) => serde_json::json!({"error":error}),
        };
        println!("{output}");
        io::stdout().flush().expect("flush output");
    }
    bridge.shutdown().await.expect("shutdown");
}
