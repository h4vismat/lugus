use desktop_host::Bridge;
use lugus_app::{ApplicationConfig, ErrorKind};
use serde_json::json;

async fn fixture() -> (tempfile::TempDir, Bridge) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("application.json"), json!({"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[]}).to_string()).unwrap();
    std::fs::write(
        dir.path().join("desktop.json"),
        json!({"application_config":"application.json"}).to_string(),
    )
    .unwrap();
    let bridge = Bridge::open(&dir.path().join("desktop.json"), false)
        .await
        .unwrap();
    (dir, bridge)
}
#[tokio::test]
async fn missing_runtime_allows_local_chats_but_rejects_send_and_unknown_fields() {
    let (dir, bridge) = fixture().await;
    assert_eq!(
        bridge.dispatch(r#"{"op":"info"}"#).await.unwrap()["runtime_available"],
        false
    );
    let local = bridge
        .dispatch(r#"{"op":"create","request":"local","title":"Offline chat"}"#)
        .await
        .unwrap();
    assert_eq!(
        bridge
            .dispatch(
                &json!({"op":"send","conversation":local["id"],"request":"send","text":"hello"})
                    .to_string()
            )
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Unavailable
    );
    assert!(
        bridge
            .dispatch(r#"{"op":"list","path":"/tmp"}"#)
            .await
            .is_err()
    );
    assert!(bridge.dispatch(&"x".repeat(1024 * 1024 + 1)).await.is_err());
    bridge.shutdown().await.unwrap();
    let app = ApplicationConfig::load(dir.path().join("application.json"))
        .await
        .unwrap()
        .open(true)
        .await
        .unwrap();
    let conversation = app
        .create_conversation("one", "Saved research")
        .await
        .unwrap();
    app.shutdown().await.unwrap();
    let reopened = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    assert_eq!(
        reopened.dispatch(r#"{"op":"list"}"#).await.unwrap()["items"][0]["id"],
        conversation.id
    );
    reopened.shutdown().await.unwrap();
}
#[tokio::test]
async fn runtime_workspace_must_exist_outside_a_repository_and_config_is_strict() {
    let (dir, bridge) = fixture().await;
    bridge.shutdown().await.unwrap();
    std::fs::create_dir(dir.path().join("runtime")).unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    let config = json!({"application_config":"application.json","runtime":{"executable":"/usr/bin/true","workspace":"runtime"}});
    std::fs::write(dir.path().join("desktop.json"), config.to_string()).unwrap();
    assert!(
        Bridge::open(&dir.path().join("desktop.json"), false)
            .await
            .is_err()
    );
    // Offline opening never touches the configured executable or starts providers.
    let offline = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    offline.shutdown().await.unwrap();
    std::fs::write(
        dir.path().join("desktop.json"),
        json!({"application_config":"application.json","unexpected":true}).to_string(),
    )
    .unwrap();
    assert!(
        Bridge::open(&dir.path().join("desktop.json"), true)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn legacy_runtime_timeout_is_accepted_without_research_cap_and_version_failure_is_reported() {
    let (dir, bridge) = fixture().await;
    bridge.shutdown().await.unwrap();
    std::fs::create_dir(dir.path().join("runtime")).unwrap();
    let path = dir.path().join("desktop.json");
    for timeout in [0, 9, 601] {
        std::fs::write(&path, json!({"application_config":"application.json","runtime":{"executable":"/usr/bin/true","workspace":"runtime","timeout_secs":timeout}}).to_string()).unwrap();
        let opened = Bridge::open(&path, false).await.unwrap();
        opened.shutdown().await.unwrap();
    }
    std::fs::write(&path, json!({"application_config":"application.json","runtime":{"executable":"/usr/bin/true","workspace":"runtime","timeout_secs":10}}).to_string()).unwrap();
    let bridge = Bridge::open(&path, false).await.unwrap();
    let c = bridge
        .dispatch(r#"{"op":"create","request":"c","title":"Configured runtime"}"#)
        .await
        .unwrap();
    let run = bridge
        .dispatch(
            &json!({"op":"send","conversation":c["id"],"request":"r","text":"hello"}).to_string(),
        )
        .await
        .unwrap();
    let status = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let status = bridge
                .dispatch(
                    &json!({"op":"status","conversation":c["id"],"run":run["id"]}).to_string(),
                )
                .await
                .unwrap();
            if status["status"] == "failed" {
                break status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    // GNU true prints a non-Codex version; BSD true may return no version output.
    assert!(matches!(
        status["error"]["kind"].as_str(),
        Some("unsupported" | "unavailable")
    ));
    assert!(
        status["error"]["message"]
            .as_str()
            .unwrap()
            .contains("CLI version")
    );
    assert!(status["error"]["message"].as_str().is_some());
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn conversation_lookup_restores_saved_chat_and_rejects_missing_id() {
    let (_dir, bridge) = fixture().await;
    let saved = bridge
        .dispatch(r#"{"op":"create","request":"saved","title":"Saved conversation"}"#)
        .await
        .unwrap();
    let restored = bridge
        .dispatch(&json!({"op":"conversation","conversation":saved["id"]}).to_string())
        .await
        .unwrap();
    assert_eq!(restored, saved);
    assert!(
        bridge
            .dispatch(r#"{"op":"conversation","conversation":"missing"}"#)
            .await
            .is_err()
    );
    bridge.shutdown().await.unwrap();
}
