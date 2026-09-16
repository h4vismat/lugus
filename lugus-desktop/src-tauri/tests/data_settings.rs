use desktop_host::Bridge;
use serde_json::{Value, json};

async fn fixture() -> (tempfile::TempDir, Bridge) {
    let dir = tempfile::tempdir().unwrap();
    let plugin = dir.path().join("sec-edgar");
    std::fs::create_dir(&plugin).unwrap();
    std::fs::write(plugin.join("plugin.json"),json!({"id":"sec-edgar","version":"0.3.0","protocol_version":1,"command":"/usr/bin/python3","args":["main.py"]}).to_string()).unwrap();
    std::fs::write(dir.path().join("application.json"),json!({"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[{"instance_id":"sec","manifest":"sec-edgar/plugin.json","active":false,"config":{"custom":"preserved"}}]}).to_string()).unwrap();
    std::fs::write(
        dir.path().join("desktop.json"),
        r#"{"application_config":"application.json"}"#,
    )
    .unwrap();
    let bridge = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    (dir, bridge)
}
async fn view(bridge: &Bridge) -> Value {
    bridge.dispatch(r#"{"op":"data_settings"}"#).await.unwrap()
}
#[tokio::test]
async fn sec_contact_settings_persist_preserve_configuration_and_require_restart() {
    let (dir, bridge) = fixture().await;
    let before = view(&bridge).await;
    assert_eq!(before["enabled"], false);
    let command = json!({"op":"save_data_settings","revision":before["revision"],"contact_name":"Test Investor","contact_email":"investor@example.test","enabled":true});
    let saved = bridge.dispatch(&command.to_string()).await.unwrap();
    assert_eq!(saved["contact_name"], "Test Investor");
    assert_eq!(saved["restart_required"], true);
    assert_eq!(saved["ready"], false);
    let config: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("application.json")).unwrap())
            .unwrap();
    assert_eq!(
        config["providers"][0]["config"]["user_agent"],
        "Lugus Test Investor investor@example.test"
    );
    assert_eq!(config["providers"][0]["config"]["custom"], "preserved");
    assert_eq!(config["financial_path"], "financial.sqlite");
    assert_eq!(
        bridge
            .dispatch(&command.to_string())
            .await
            .unwrap_err()
            .kind,
        lugus_app::ErrorKind::Conflict
    );
    bridge.shutdown().await.unwrap();
    drop(bridge);
    let reopened = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    assert_eq!(
        view(&reopened).await["contact_email"],
        "investor@example.test"
    );
    assert_eq!(view(&reopened).await["restart_required"], false);
    reopened.shutdown().await.unwrap();
}
#[tokio::test]
async fn invalid_identity_does_not_change_config_and_renderer_cannot_choose_executables() {
    let (dir, bridge) = fixture().await;
    let before = std::fs::read(dir.path().join("application.json")).unwrap();
    let revision = view(&bridge).await["revision"].clone();
    for (name, email) in [
        ("", "a@example.test"),
        ("Test", "not-an-email"),
        ("Test\r\nInjected", "a@example.test"),
    ] {
        assert!(bridge.dispatch(&json!({"op":"save_data_settings","revision":revision,"contact_name":name,"contact_email":email,"enabled":true}).to_string()).await.is_err());
        assert_eq!(
            std::fs::read(dir.path().join("application.json")).unwrap(),
            before
        );
    }
    assert!(bridge.dispatch(&json!({"op":"save_data_settings","revision":revision,"contact_name":"Test","contact_email":"a@example.test","enabled":true,"manifest":"/tmp/evil"}).to_string()).await.is_err());
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn saved_provider_becomes_ready_only_after_restart() {
    let (dir, bridge) = fixture().await;
    bridge.shutdown().await.unwrap();
    drop(bridge);
    let manifest = dir.path().join("sec-edgar/plugin.json");
    let worker = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/provider.py")
        .canonicalize()
        .unwrap();
    std::fs::write(&manifest,json!({"id":"sec-edgar","version":"1","protocol_version":1,"command":"/usr/bin/python3","args":[worker]}).to_string()).unwrap();
    let path = dir.path().join("application.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["providers"][0]["config"] =
        json!({"mode":"apple","plugin_id":"sec-edgar","barrier":dir.path().join("provider")});
    std::fs::write(&path, config.to_string()).unwrap();
    let bridge = Bridge::open(&dir.path().join("desktop.json"), false)
        .await
        .unwrap();
    let before = view(&bridge).await;
    let saved=bridge.dispatch(&json!({"op":"save_data_settings","revision":before["revision"],"contact_name":"Test Investor","contact_email":"investor@example.test","enabled":true}).to_string()).await.unwrap();
    assert_eq!(saved["ready"], false);
    assert_eq!(saved["restart_required"], true);
    bridge.shutdown().await.unwrap();
    drop(bridge);
    let reopened = Bridge::open(&dir.path().join("desktop.json"), false)
        .await
        .unwrap();
    let actual = view(&reopened).await;
    assert_eq!(actual["ready"], true);
    assert_eq!(actual["restart_required"], false);
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn multiple_sec_instances_do_not_block_application_startup() {
    let (dir, bridge) = fixture().await;
    bridge.shutdown().await.unwrap();
    drop(bridge);
    let path = dir.path().join("application.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut duplicate = config["providers"][0].clone();
    duplicate["instance_id"] = json!("second-sec");
    config["providers"].as_array_mut().unwrap().push(duplicate);
    std::fs::write(&path, config.to_string()).unwrap();
    let bridge = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    assert_eq!(
        bridge
            .dispatch(r#"{"op":"data_settings"}"#)
            .await
            .unwrap_err()
            .kind,
        lugus_app::ErrorKind::Conflict
    );
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn settings_save_respects_configuration_writer_lock() {
    let (dir, bridge) = fixture().await;
    let before = view(&bridge).await;
    let lock =
        std::fs::File::create(dir.path().join("application.json.data-settings.lock")).unwrap();
    lock.lock().unwrap();
    let command = json!({"op":"save_data_settings","revision":before["revision"],"contact_name":"Test Investor","contact_email":"investor@example.test","enabled":true});
    assert_eq!(
        bridge
            .dispatch(&command.to_string())
            .await
            .unwrap_err()
            .kind,
        lugus_app::ErrorKind::Conflict
    );
    lock.unlock().unwrap();
    bridge.dispatch(&command.to_string()).await.unwrap();
    bridge.shutdown().await.unwrap();
}
