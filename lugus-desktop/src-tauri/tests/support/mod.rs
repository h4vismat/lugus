//! Explicit synthetic runtime shared only by tests and the QA seed example.
#![allow(dead_code)]
use desktop_host::Bridge;
use lugus_agent::{
    AgentRuntime, RunOutcome, RunReport, RunRequest, RuntimeEvent, ToolCall, ToolExecutor,
};
use lugus_app::{conversations::*, *};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};

#[derive(Clone, Default)]
pub struct Factory {
    pub prompts: Arc<Mutex<Vec<String>>>,
    pub hold: bool,
    pub fail: bool,
    pub views: Arc<Mutex<Vec<String>>>,
}
#[async_trait::async_trait]
impl RuntimeFactory for Factory {
    async fn create(&self) -> Result<Box<dyn AgentRuntime>> {
        Ok(Box::new(self.clone()))
    }
}
async fn call(
    tools: &dyn ToolExecutor,
    run: &str,
    id: &str,
    name: &str,
    arguments: Value,
) -> Value {
    let result = tools
        .execute(ToolCall {
            run_id: run.into(),
            call_id: id.into(),
            name: name.into(),
            arguments,
        })
        .await;
    assert!(result.success, "{name}: {}", result.content);
    serde_json::from_str(&result.content).unwrap()
}
#[async_trait::async_trait]
impl AgentRuntime for Factory {
    async fn run(
        &mut self,
        request: RunRequest,
        tools: &dyn ToolExecutor,
        events: mpsc::Sender<RuntimeEvent>,
        _: watch::Receiver<bool>,
    ) -> lugus_agent::Result<RunReport> {
        if self.hold {
            std::future::pending::<()>().await;
        }
        if self.fail {
            return Err(lugus_agent::Error::Protocol("explicit test failure".into()));
        }
        if request.tools.is_empty() {
            assert!(!request.allow_web_search);
            return Ok(RunReport { run_id:request.run_id, outcome:RunOutcome::Completed,
                final_text: json!({"workflow":"research","subjects":[{"text":"AAPL","exchange":null}],"start":"2024-01-01","end":"2024-12-31","clarification":null}).to_string() });
        }
        self.prompts.lock().unwrap().push(request.prompt.clone());
        events
            .send(RuntimeEvent::TextDelta {
                text: "Reading saved evidence".into(),
            })
            .await
            .unwrap();
        let prepared: lugus_app::research::PreparedResearch =
            serde_json::from_str(&request.context).unwrap();
        assert!(prepared.issues.is_empty(), "{:?}", prepared.issues);
        assert!(!request.allow_web_search);
        assert!(
            !request
                .tools
                .iter()
                .any(|s| s.name.starts_with("lugus_fetch_"))
        );
        self.views
            .lock()
            .unwrap()
            .extend(prepared.views.iter().map(|v| v.id.clone()));
        let prices = prepared
            .datasets
            .iter()
            .find(|d| d.purpose == "daily_close")
            .unwrap();
        let page = call(
            tools,
            &request.run_id,
            "read-data",
            "lugus_read_dataset",
            json!({"dataset_id":prices.page.header.id,"page":{"offset":0,"limit":10}}),
        )
        .await;
        assert!(page.to_string().contains("101"));
        Ok(RunReport {
            run_id: request.run_id,
            outcome: RunOutcome::Completed,
            final_text: "The saved price and reported facts are available.".into(),
        })
    }
    async fn close(&mut self) -> lugus_agent::Result<()> {
        Ok(())
    }
}
pub async fn setup(factory: Factory) -> (tempfile::TempDir, Bridge, ConversationHost) {
    let root = tempfile::tempdir().unwrap();
    let (bridge, host) = setup_in(root.path(), factory).await;
    (root, bridge, host)
}
pub async fn setup_in(root: &std::path::Path, factory: Factory) -> (Bridge, ConversationHost) {
    let worker = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/provider.py")
        .canonicalize()
        .unwrap();
    std::fs::write(root.join("manifest.json"), json!({"id":"yfinance","version":"1","protocol_version":1,"command":"python3","args":[worker]}).to_string()).unwrap();
    std::fs::write(root.join("application.json"), json!({"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[{"instance_id":"fixture","manifest":"manifest.json","active":true,"config":{"mode":"apple","plugin_id":"yfinance","barrier":root.join("provider")}}]}).to_string()).unwrap();
    std::fs::write(
        root.join("desktop.json"),
        json!({"application_config":"application.json"}).to_string(),
    )
    .unwrap();
    let app = ApplicationConfig::load(root.join("application.json"))
        .await
        .unwrap()
        .open(false)
        .await
        .unwrap();
    let host = ConversationHost::start(app, Arc::new(factory), ConversationOptions::default())
        .await
        .unwrap();
    (Bridge::from_host(host.clone(), true), host)
}
pub async fn dispatch(bridge: &Bridge, command: Value) -> Value {
    bridge.dispatch(&command.to_string()).await.unwrap()
}
