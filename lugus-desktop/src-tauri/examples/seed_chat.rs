//! Explicit QA seed command, excluded from the native application binary.
#[path = "../tests/support/mod.rs"]
mod support;
use lugus_app::conversations::RunStatus;
use serde_json::json;

#[tokio::main]
async fn main() {
    let target = std::env::args_os()
        .nth(1)
        .expect("usage: seed_chat TARGET_DIR (new directory)");
    let root = std::path::PathBuf::from(target);
    std::fs::create_dir(&root).expect("TARGET_DIR must not already exist");
    let root = root.canonicalize().unwrap();
    let (bridge, host) = support::setup_in(&root, support::Factory::default()).await;
    let conversation = support::dispatch(
        &bridge,
        json!({"op":"create","request":"qa-company","title":"Apple · synthetic QA research"}),
    )
    .await;
    let id = conversation["id"].as_str().unwrap();
    let run = support::dispatch(&bridge, json!({"op":"send","conversation":id,"request":"qa-first-turn","text":"Using the explicit synthetic QA provider, show Apple's daily prices and reported assets. These values are test data."})).await;
    let terminal = host.wait(id, run["id"].as_str().unwrap()).await.unwrap();
    assert_eq!(
        terminal.status,
        RunStatus::Completed,
        "{:?}",
        terminal.error
    );
    let workspace = support::dispatch(&bridge, json!({"op":"workspace","conversation":id})).await;
    assert_eq!(workspace["view_ids"].as_array().unwrap().len(), 2);
    let data = support::dispatch(
        &bridge,
        json!({"op":"read","conversation":id,"view":workspace["view_ids"][0]}),
    )
    .await;
    assert_eq!(data["rows"].as_array().unwrap().len(), 10);
    assert!(data["header"]["binding_id"].is_string());
    bridge.shutdown().await.unwrap();
    println!(
        "Synthetic QA conversation and evidence persisted. Offline desktop config: {}",
        root.join("desktop.json").display()
    );
}
