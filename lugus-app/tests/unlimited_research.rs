use lugus_agent::{
    AgentRuntime, RunOutcome, RunReport, RunRequest, RuntimeEvent, ToolCall, ToolExecutor,
};
use lugus_app::{conversations::*, *};
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};
#[derive(Clone)]
struct Runtime {
    hold: bool,
}
#[async_trait::async_trait]
impl RuntimeFactory for Runtime {
    async fn create(&self) -> Result<Box<dyn AgentRuntime>> {
        Ok(Box::new(self.clone()))
    }
}
#[async_trait::async_trait]
impl AgentRuntime for Runtime {
    async fn run(
        &mut self,
        request: RunRequest,
        tools: &dyn ToolExecutor,
        events: mpsc::Sender<RuntimeEvent>,
        _: watch::Receiver<bool>,
    ) -> lugus_agent::Result<RunReport> {
        assert_eq!(request.limits.timeout, Duration::MAX);
        if self.hold {
            std::future::pending::<()>().await;
        }
        assert!(request.prompt.len() > 1024 * 1024);
        for n in 0..140 {
            let result = tools
                .execute(ToolCall {
                    run_id: request.run_id.clone(),
                    call_id: format!("call-{n}"),
                    name: "lugus_dataset_header".into(),
                    arguments: serde_json::json!({"dataset_id":"missing"}),
                })
                .await;
            assert!(!result.success);
            assert!(
                !result.content.contains("resource_limit"),
                "{}",
                result.content
            );
        }
        for _ in 0..600 {
            events
                .send(RuntimeEvent::TextDelta {
                    text: "x".repeat(4096),
                })
                .await
                .unwrap();
        }
        Ok(RunReport {
            run_id: request.run_id,
            outcome: RunOutcome::Completed,
            final_text: "x".repeat(2 * 1024 * 1024),
        })
    }
    async fn close(&mut self) -> lugus_agent::Result<()> {
        Ok(())
    }
}
async fn setup(hold: bool) -> (tempfile::TempDir, ConversationHost, Conversation, RunRecord) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("application.json");
    std::fs::write(&path, r#"{"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[]}"#).unwrap();
    let mut config = ApplicationConfig::load(&path).await.unwrap();
    config.limits = Limits::unlimited_research();
    config.conversation_limits = Some(ConversationLimits::unlimited_research());
    let app = config.open(true).await.unwrap();
    let host = ConversationHost::start_with_tools(
        app,
        Arc::new(Runtime { hold }),
        ConversationOptions {
            run_limits: lugus_agent::RunLimits {
                timeout: Duration::MAX,
                max_tool_calls: isize::MAX as usize,
                max_tool_result_bytes: isize::MAX as usize,
            },
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let c = host.create("create", "Unlimited research").await.unwrap();
    let run = host
        .send(SendMessageRequest {
            conversation_id: c.id.clone(),
            request_id: "large".into(),
            text: "q".repeat(2 * 1024 * 1024),
            company_hint: None,
            research_brief: None,
            selected: vec![],
        })
        .await
        .unwrap();
    (dir, host, c, run)
}
#[tokio::test]
async fn large_research_messages_outputs_activity_and_tool_counts_have_no_desktop_budget() {
    let (_dir, host, c, run) = setup(false).await;
    let result = tokio::time::timeout(Duration::from_secs(20), host.wait(&c.id, &run.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.status, RunStatus::Completed, "{result:?}");
    let page = host
        .messages(
            &c.id,
            PageRequest {
                offset: 0,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items.last().unwrap().text.len(), 2 * 1024 * 1024);
    let tools = host
        .tool_records(
            &c.id,
            &run.id,
            PageRequest {
                offset: 100,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert_eq!(tools.items.len(), 40);
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn unlimited_execution_remains_cancellable() {
    let (_dir, host, c, run) = setup(true).await;
    host.cancel(&c.id, &run.id).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), host.wait(&c.id, &run.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.status, RunStatus::Interrupted);
    host.shutdown().await.unwrap();
}
