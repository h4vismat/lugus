use desktop_host::Bridge;
use lugus_agent::{AgentRuntime, RunOutcome, RunReport, RunRequest, RuntimeEvent, ToolExecutor};
use lugus_app::{ApplicationConfig, ErrorKind, Result, conversations::*};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};

async fn fixture() -> (tempfile::TempDir, Bridge) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("runtime")).unwrap();
    std::fs::write(
        dir.path().join("app.json"),
        r#"{"financial_path":"financial.sqlite","application_path":"app.sqlite","providers":[]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("desktop.json"),
        json!({
            "application_config":"app.json",
            "agents": {
                "codex":{"executable":"/usr/bin/true","workspace":"runtime"},
                "claude_code":{"executable":"/usr/bin/true","workspace":"runtime"}
            }
        })
        .to_string(),
    )
    .unwrap();
    let bridge = Bridge::open(&dir.path().join("desktop.json"), false)
        .await
        .unwrap();
    (dir, bridge)
}

async fn web_settings(bridge: &Bridge) -> Value {
    bridge
        .dispatch(r#"{"op":"web_search_settings"}"#)
        .await
        .unwrap()
}

#[tokio::test]
async fn search_preference_persists_independently_of_sec_and_agent_selection() {
    let (dir, bridge) = fixture().await;
    let original = std::fs::read(dir.path().join("app.json")).unwrap();
    assert_eq!(web_settings(&bridge).await["enabled"], false);
    let saved = bridge
        .dispatch(r#"{"op":"save_web_search_settings","enabled":true}"#)
        .await
        .unwrap();
    assert_eq!(saved["enabled"], true);
    assert_eq!(
        saved["runtime_available"],
        true,
        "{:?}",
        bridge.dispatch(r#"{"op":"agent_settings"}"#).await
    );
    bridge
        .dispatch(r#"{"op":"select_agent","agent":"claude_code"}"#)
        .await
        .unwrap();
    assert_eq!(web_settings(&bridge).await["enabled"], true);
    assert_eq!(
        std::fs::read(dir.path().join("app.json")).unwrap(),
        original
    );
    bridge.shutdown().await.unwrap();
    let reopened = Bridge::open(&dir.path().join("desktop.json"), false)
        .await
        .unwrap();
    assert_eq!(web_settings(&reopened).await["enabled"], true);
    assert_eq!(web_settings(&reopened).await["agent"], "claude_code");
    reopened
        .dispatch(r#"{"op":"save_web_search_settings","enabled":false}"#)
        .await
        .unwrap();
    assert_eq!(web_settings(&reopened).await["enabled"], false);
    assert_eq!(web_settings(&reopened).await["agent"], "claude_code");
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_search_save_keeps_memory_and_offline_never_reports_availability() {
    let (dir, bridge) = fixture().await;
    std::fs::create_dir(dir.path().join("desktop.agents.json")).unwrap();
    assert!(
        bridge
            .dispatch(r#"{"op":"save_web_search_settings","enabled":true}"#)
            .await
            .is_err()
    );
    assert_eq!(web_settings(&bridge).await["enabled"], false);
    bridge.shutdown().await.unwrap();
    std::fs::remove_dir(dir.path().join("desktop.agents.json")).unwrap();
    std::fs::write(
        dir.path().join("desktop.agents.json"),
        r#"{"selected":"codex","allow_web_search":true}"#,
    )
    .unwrap();
    let offline = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    let view = web_settings(&offline).await;
    assert_eq!(view["enabled"], true);
    assert_eq!(view["offline"], true);
    assert_eq!(view["runtime_available"], false);
    offline.shutdown().await.unwrap();
}

#[derive(Clone)]
struct Model {
    requests: Arc<Mutex<Vec<RunRequest>>>,
    workflow: &'static str,
    unavailable: bool,
}
#[async_trait::async_trait]
impl RuntimeFactory for Model {
    async fn create(&self) -> Result<Box<dyn AgentRuntime>> {
        Ok(Box::new(self.clone()))
    }
}
#[async_trait::async_trait]
impl AgentRuntime for Model {
    async fn run(
        &mut self,
        request: RunRequest,
        _: &dyn ToolExecutor,
        _: mpsc::Sender<RuntimeEvent>,
        _: watch::Receiver<bool>,
    ) -> lugus_agent::Result<RunReport> {
        self.requests.lock().unwrap().push(request.clone());
        let final_text = if request.tools.is_empty() {
            json!({"workflow":self.workflow,"subjects":[],"start":null,"end":null,"clarification":null}).to_string()
        } else if self.unavailable {
            return Err(lugus_agent::Error::NeedsAttention(
                "Search is unavailable for this provider".into(),
            ));
        } else {
            "Fixture answer with [source](https://example.test/news).".into()
        };
        Ok(RunReport {
            run_id: request.run_id,
            outcome: RunOutcome::Completed,
            final_text,
        })
    }
    async fn close(&mut self) -> lugus_agent::Result<()> {
        Ok(())
    }
}

async fn run_search(
    enabled: bool,
    workflow: &'static str,
    unavailable: bool,
) -> (RunRecord, Vec<RunRequest>, Value) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.json");
    std::fs::write(
        &path,
        r#"{"financial_path":"financial.sqlite","application_path":"app.sqlite","providers":[]}"#,
    )
    .unwrap();
    let app = ApplicationConfig::load(path)
        .await
        .unwrap()
        .open(false)
        .await
        .unwrap();
    let model = Model {
        requests: Default::default(),
        workflow,
        unavailable,
    };
    let host = ConversationHost::start(
        app.clone(),
        Arc::new(model.clone()),
        ConversationOptions {
            allow_web_search: enabled,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let chat = host.create("create", "Web research").await.unwrap();
    let run = host
        .send(SendMessageRequest {
            research_brief: None,
            company_hint: None,
            conversation_id: chat.id.clone(),
            request_id: "search".into(),
            text: "Search the internet for the latest semiconductor news".into(),
            selected: vec![],
        })
        .await
        .unwrap();
    let completed = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        host.wait(&chat.id, &run.id),
    )
    .await
    .unwrap()
    .unwrap();
    let preparation = app
        .conversation_preparation(&chat.id, &run.id)
        .await
        .unwrap();
    let saved = preparation
        .map(|text| serde_json::from_str(&text).unwrap())
        .unwrap_or(Value::Null);
    let requests = model.requests.lock().unwrap().clone();
    host.shutdown().await.unwrap();
    (completed, requests, saved)
}

#[tokio::test]
async fn web_requests_reach_analysis_without_requiring_a_financial_provider() {
    let (run, requests, saved) = run_search(true, "web_search", false).await;
    assert_eq!(run.status, RunStatus::Completed, "{:?}", run.error);
    assert_eq!(requests.len(), 2);
    assert!(
        !requests[0].allow_web_search,
        "interpretation must remain tool-free"
    );
    assert!(requests[1].allow_web_search);
    assert!(
        requests[1]
            .tools
            .iter()
            .all(|t| !t.name.starts_with("lugus_fetch_"))
    );
    assert_eq!(saved["intent"]["workflow"], "web_search");
    assert_eq!(saved["fetches"], json!([]));
}

#[tokio::test]
async fn disabled_web_requests_return_guidance_without_running_an_answer_session() {
    let (run, requests, saved) = run_search(false, "web_search", false).await;
    assert_eq!(run.status, RunStatus::Completed, "{:?}", run.error);
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].allow_web_search);
    assert!(
        saved["clarification"]
            .as_str()
            .unwrap()
            .contains("Internet search")
    );
}

#[tokio::test]
async fn ordinary_analysis_respects_the_search_toggle() {
    for enabled in [false, true] {
        let (run, requests, _) = run_search(enabled, "conversation", false).await;
        assert_eq!(run.status, RunStatus::Completed, "{:?}", run.error);
        assert!(!requests[0].allow_web_search);
        assert_eq!(requests[1].allow_web_search, enabled);
    }
}

#[tokio::test]
async fn unavailable_runtime_does_not_report_a_successful_search() {
    let (run, requests, saved) = run_search(true, "web_search", true).await;
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error.unwrap().kind, ErrorKind::NeedsAttention);
    assert_eq!(requests.len(), 2);
    assert_eq!(saved["fetches"], json!([]));
}

#[cfg(unix)]
#[tokio::test]
async fn saved_toggle_reaches_native_cli_and_is_frozen_for_an_in_progress_message() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, bridge) = fixture().await;
    bridge.shutdown().await.unwrap();
    let peer = dir.path().join("claude-fixture");
    std::fs::write(&peer, include_str!("fixtures/web_search_claude.py")).unwrap();
    std::fs::set_permissions(&peer, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.path().join("desktop.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["agents"]["claude_code"]["executable"] = json!(peer);
    std::fs::write(&path, config.to_string()).unwrap();
    let bridge = Bridge::open(&path, false).await.unwrap();
    bridge
        .dispatch(r#"{"op":"select_agent","agent":"claude_code"}"#)
        .await
        .unwrap();
    bridge
        .dispatch(r#"{"op":"save_web_search_settings","enabled":true}"#)
        .await
        .unwrap();
    let chat = bridge
        .dispatch(r#"{"op":"create","request":"create","title":"Native search"}"#)
        .await
        .unwrap();
    let workspace = dir.path().join("runtime");
    std::fs::write(workspace.join("pause"), "").unwrap();
    let send = |id, text| {
        json!({"op":"send","conversation":chat["id"],"request":id,"text":text}).to_string()
    };
    let first = bridge
        .dispatch(&send("first", "Search the internet for news"))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !workspace.join("interpreting").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    bridge
        .dispatch(r#"{"op":"save_web_search_settings","enabled":false}"#)
        .await
        .unwrap();
    std::fs::remove_file(workspace.join("pause")).unwrap();
    wait_for_run(&bridge, &chat["id"], &first["id"]).await;
    let second = bridge
        .dispatch(&send("second", "An ordinary conversation"))
        .await
        .unwrap();
    wait_for_run(&bridge, &chat["id"], &second["id"]).await;
    let third = bridge
        .dispatch(&send("third", "Search the internet again"))
        .await
        .unwrap();
    wait_for_run(&bridge, &chat["id"], &third["id"]).await;
    let sessions: Vec<Value> = std::fs::read_to_string(workspace.join("search-sessions.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        sessions.len(),
        5,
        "disabled web request must not start an answer session"
    );
    assert_eq!(sessions[0]["builtins"], "");
    assert_eq!(
        sessions[1]["builtins"], "WebSearch,WebFetch",
        "in-progress message must retain its search permission"
    );
    assert!(
        sessions[1]["allowed"]
            .as_str()
            .unwrap()
            .split(',')
            .any(|tool| tool == "WebSearch")
    );
    assert_eq!(sessions[2]["builtins"], "");
    assert_eq!(
        sessions[3]["builtins"], "",
        "next answer must use the newly disabled setting"
    );
    assert!(
        !sessions[3]["allowed"]
            .as_str()
            .unwrap()
            .split(',')
            .any(|tool| tool == "WebSearch")
    );
    assert_eq!(sessions[4]["builtins"], "");
    let messages = bridge
        .dispatch(&json!({"op":"messages","conversation":chat["id"]}).to_string())
        .await
        .unwrap();
    assert!(
        messages["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["text"] == "Fixture answer with [source](https://example.test/news).")
    );
    assert!(messages["items"].as_array().unwrap().iter().any(|m| {
        m["text"]
            .as_str()
            .is_some_and(|text| text.contains("Internet search is disabled"))
    }));
    bridge.shutdown().await.unwrap();
}

async fn wait_for_run(bridge: &Bridge, chat: &Value, run: &Value) {
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
                assert_eq!(state["status"], "completed", "{state}");
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
