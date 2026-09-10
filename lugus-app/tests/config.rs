use lugus_app::*;
use serde_json::json;
#[tokio::test]
async fn duplicate_external_instances_fail_before_database_creation() {
    let root = tempfile::tempdir().unwrap();
    let manifest = root.path().join("manifest.json");
    std::fs::write(&manifest, json!({"id":"fixture","version":"1","protocol_version":1,"command":"must-not-run","args":[]}).to_string()).unwrap();
    let provider =
        json!({"instance_id":"same","manifest":"manifest.json","active":true,"config":{}});
    let path = root.path().join("config.json");
    std::fs::write(&path, json!({"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[provider,provider]}).to_string()).unwrap();
    let config = ApplicationConfig::load(&path).await.unwrap();
    assert_eq!(
        config.open(false).await.err().unwrap().kind,
        ErrorKind::Conflict
    );
    assert!(!root.path().join("financial.sqlite").exists());
    assert!(!root.path().join("application.sqlite").exists());
}
#[tokio::test]
async fn configuration_load_is_strict_and_offline_never_starts_process() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("manifest.json"), json!({"id":"fixture","version":"1","protocol_version":1,"command":"must-not-run","args":[]}).to_string()).unwrap();
    let path = root.path().join("config.json");
    let mut value = json!({"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[{"instance_id":"one","manifest":"manifest.json","active":true,"config":{}}]});
    value["unknown"] = json!(true);
    std::fs::write(&path, value.to_string()).unwrap();
    assert_eq!(
        ApplicationConfig::load(&path).await.err().unwrap().kind,
        ErrorKind::InvalidInput
    );
    value.as_object_mut().unwrap().remove("unknown");
    std::fs::write(&path, value.to_string()).unwrap();
    let config = ApplicationConfig::load(&path).await.unwrap();
    std::fs::remove_file(root.path().join("manifest.json")).unwrap();
    let app = config.open(true).await.unwrap();
    assert!(app.offering().unwrap().operations().next().is_none());
    assert!(app.providers().unwrap().is_empty());
    app.shutdown().await.unwrap();
}
#[tokio::test]
async fn optional_conversation_limits_load_persisted_config_and_reject_explicit_changes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.json");
    let mut value = json!({"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[],"conversation_limits":{"active_runs":2}});
    std::fs::write(&path, value.to_string()).unwrap();
    let app = ApplicationConfig::load(&path)
        .await
        .unwrap()
        .open(true)
        .await
        .unwrap();
    assert_eq!(app.conversation_limits().await.unwrap().active_runs, 2);
    let c = app
        .create_conversation("create", "Offline conversation")
        .await
        .unwrap();
    app.shutdown().await.unwrap();
    value.as_object_mut().unwrap().remove("conversation_limits");
    std::fs::write(&path, value.to_string()).unwrap();
    let app = ApplicationConfig::load(&path)
        .await
        .unwrap()
        .open(true)
        .await
        .unwrap();
    assert_eq!(app.conversation_limits().await.unwrap().active_runs, 2);
    assert_eq!(app.conversation(&c.id).await.unwrap(), c);
    app.shutdown().await.unwrap();
    value["conversation_limits"] = json!({"active_runs":3});
    std::fs::write(&path, value.to_string()).unwrap();
    assert_eq!(
        ApplicationConfig::load(&path)
            .await
            .unwrap()
            .open(true)
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::Conflict
    );
    value["conversation_limits"] = json!({"runtime_close_timeout_ms":0});
    std::fs::write(&path, value.to_string()).unwrap();
    assert_eq!(
        ApplicationConfig::load(&path).await.err().unwrap().kind,
        ErrorKind::InvalidInput
    );
}
