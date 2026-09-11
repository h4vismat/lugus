use desktop_host::Bridge;
use lugus_app::ErrorKind;
use serde_json::{Value, json};

async fn fixture() -> (tempfile::TempDir, Bridge) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("runtime")).unwrap();
    std::fs::write(dir.path().join("application.json"), json!({"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[]}).to_string()).unwrap();
    std::fs::write(
        dir.path().join("desktop.json"),
        json!({
            "application_config":"application.json",
            "runtime":{"executable":"/usr/bin/true","workspace":"runtime","model":"legacy-model"},
            "agents":{"claude_code":{"executable":"/usr/bin/true","workspace":"runtime"}}
        })
        .to_string(),
    )
    .unwrap();
    let bridge = Bridge::open(&dir.path().join("desktop.json"), false)
        .await
        .unwrap();
    (dir, bridge)
}
async fn settings(bridge: &Bridge) -> Value {
    bridge.dispatch(r#"{"op":"agent_settings"}"#).await.unwrap()
}

#[tokio::test]
async fn selection_persists_without_rewriting_legacy_configuration() {
    let (dir, bridge) = fixture().await;
    let path = dir.path().join("desktop.json");
    let original = std::fs::read(&path).unwrap();
    let before = settings(&bridge).await;
    assert_eq!(before["selected"], "codex");
    assert_eq!(before["agents"][0]["available"], true);
    assert_eq!(before["agents"][1]["available"], true);
    let saved = bridge
        .dispatch(r#"{"op":"select_agent","agent":"claude_code"}"#)
        .await
        .unwrap();
    assert_eq!(saved["selected"], "claude_code");
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(
        bridge.dispatch(r#"{"op":"info"}"#).await.unwrap()["agent"],
        "claude_code"
    );
    bridge.shutdown().await.unwrap();
    let reopened = Bridge::open(&path, false).await.unwrap();
    assert_eq!(settings(&reopened).await["selected"], "claude_code");
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_selection_and_renderer_executable_overrides_are_rejected() {
    let (_dir, bridge) = fixture().await;
    for payload in [
        r#"{"op":"select_agent","agent":"unknown"}"#,
        r#"{"op":"select_agent","agent":"claude_code","executable":"/bin/sh"}"#,
    ] {
        assert_eq!(
            bridge.dispatch(payload).await.unwrap_err().kind,
            ErrorKind::InvalidInput
        );
    }
    assert_eq!(settings(&bridge).await["selected"], "codex");
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn unavailable_profile_does_not_break_codex_or_change_saved_selection() {
    let (dir, bridge) = fixture().await;
    bridge.shutdown().await.unwrap();
    let path = dir.path().join("desktop.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["agents"]["claude_code"]["executable"] = json!("missing-claude");
    std::fs::write(&path, config.to_string()).unwrap();
    let bridge = Bridge::open(&path, false).await.unwrap();
    assert_eq!(settings(&bridge).await["agents"][1]["available"], false);
    assert_eq!(
        bridge
            .dispatch(r#"{"op":"select_agent","agent":"claude_code"}"#)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Unavailable
    );
    assert_eq!(settings(&bridge).await["selected"], "codex");
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn offline_mode_never_enables_an_agent_from_settings() {
    let (dir, bridge) = fixture().await;
    bridge.shutdown().await.unwrap();
    let bridge = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    let view = settings(&bridge).await;
    assert_eq!(view["offline"], true);
    assert!(
        view["agents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["available"] == false)
    );
    assert_eq!(
        bridge
            .dispatch(r#"{"op":"select_agent","agent":"claude_code"}"#)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Unavailable
    );
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_persistence_keeps_the_previous_selection() {
    let (dir, bridge) = fixture().await;
    std::fs::create_dir(dir.path().join("desktop.agents.json")).unwrap();
    assert!(
        bridge
            .dispatch(r#"{"op":"select_agent","agent":"claude_code"}"#)
            .await
            .is_err()
    );
    assert_eq!(settings(&bridge).await["selected"], "codex");
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn missing_legacy_codex_executable_still_allows_selecting_claude() {
    let (dir, bridge) = fixture().await;
    bridge.shutdown().await.unwrap();
    let path = dir.path().join("desktop.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["runtime"]["executable"] = json!("removed-codex");
    std::fs::write(&path, config.to_string()).unwrap();
    std::fs::write(
        dir.path().join("desktop.agents.json"),
        r#"{"selected":"codex"}"#,
    )
    .unwrap();
    let bridge = Bridge::open(&path, false).await.unwrap();
    let before = settings(&bridge).await;
    assert_eq!(before["runtime_available"], false);
    assert_eq!(before["agents"][0]["available"], false);
    assert_eq!(before["agents"][1]["available"], true);
    let saved = bridge
        .dispatch(r#"{"op":"select_agent","agent":"claude_code"}"#)
        .await
        .unwrap();
    assert_eq!(saved["selected"], "claude_code");
    assert_eq!(saved["runtime_available"], true);
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn selected_claude_profile_runs_both_sessions_and_switching_back_uses_codex() {
    let (dir, bridge) = fixture().await;
    bridge.shutdown().await.unwrap();
    let path = dir.path().join("desktop.json");
    let executable = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/claude.py")
        .canonicalize()
        .unwrap();
    let mut config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["agents"]["claude_code"]["executable"] = json!(executable);
    std::fs::write(&path, config.to_string()).unwrap();
    let bridge = Bridge::open(&path, false).await.unwrap();
    bridge
        .dispatch(r#"{"op":"select_agent","agent":"claude_code"}"#)
        .await
        .unwrap();
    let chat = bridge
        .dispatch(r#"{"op":"create","request":"create","title":"Agent routing"}"#)
        .await
        .unwrap();
    let send =
        |id| json!({"op":"send","conversation":chat["id"],"request":id,"text":"Hello"}).to_string();
    let run = bridge.dispatch(&send("claude")).await.unwrap();
    let completed = wait_for_run(&bridge, &chat["id"], &run["id"]).await;
    assert_eq!(completed["status"], "completed", "{completed}");
    let messages = bridge
        .dispatch(&json!({"op":"messages","conversation":chat["id"]}).to_string())
        .await
        .unwrap();
    assert!(
        messages["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["text"] == "Claude fixture answer")
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("runtime/sessions")).unwrap(),
        "interpretation\nanswer\n"
    );
    bridge
        .dispatch(r#"{"op":"select_agent","agent":"codex"}"#)
        .await
        .unwrap();
    let next = bridge.dispatch(&send("codex")).await.unwrap();
    let failed = wait_for_run(&bridge, &chat["id"], &next["id"]).await;
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["error"]["kind"], "unavailable", "{failed}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("runtime/sessions")).unwrap(),
        "interpretation\nanswer\n"
    );
    bridge.shutdown().await.unwrap();
}

async fn wait_for_run(bridge: &Bridge, chat: &Value, run: &Value) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let state = bridge
                .dispatch(&json!({"op":"status","conversation":chat,"run":run}).to_string())
                .await
                .unwrap();
            if matches!(
                state["status"].as_str(),
                Some("completed" | "failed" | "interrupted")
            ) {
                return state;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
