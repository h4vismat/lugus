mod support;
use lugus_agent::{
    AgentRuntime, RunOutcome, RunReport, RunRequest, RuntimeEvent, ToolCall, ToolExecutor,
};
use lugus_app::{
    conversations::*,
    portfolio::*,
    research::{Interpreter, ResearchIntent},
    *,
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{mpsc, watch};
struct NoResearch(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl Interpreter for NoResearch {
    async fn interpret(
        &self,
        _: &RunRecord,
        _: Option<&str>,
        _: watch::Receiver<bool>,
    ) -> Result<ResearchIntent> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(AppError::new(
            ErrorKind::Unsupported,
            "company interpretation must not run",
            false,
        ))
    }
}
#[derive(Clone)]
struct Reader {
    selected: String,
    unselected: String,
}
#[async_trait::async_trait]
impl RuntimeFactory for Reader {
    async fn create(&self) -> Result<Box<dyn AgentRuntime>> {
        Ok(Box::new(self.clone()))
    }
}
#[async_trait::async_trait]
impl AgentRuntime for Reader {
    async fn run(
        &mut self,
        r: RunRequest,
        t: &dyn ToolExecutor,
        _: mpsc::Sender<RuntimeEvent>,
        _: watch::Receiver<bool>,
    ) -> lugus_agent::Result<RunReport> {
        assert!(!r.allow_web_search);
        assert!(!r.tools.iter().any(|t| t.name.starts_with("lugus_fetch")));
        let allowed = t
            .execute(ToolCall {
                run_id: r.run_id.clone(),
                call_id: "read".into(),
                name: "lugus_read_portfolio_snapshot".into(),
                arguments: json!({"snapshot_id":self.selected}),
            })
            .await;
        assert!(allowed.success, "{}", allowed.content);
        let value: serde_json::Value = serde_json::from_str(&allowed.content).unwrap();
        assert_eq!(value["summary"]["name"], "Original");
        let denied = t
            .execute(ToolCall {
                run_id: r.run_id.clone(),
                call_id: "denied".into(),
                name: "lugus_read_portfolio_snapshot".into(),
                arguments: json!({"snapshot_id":self.unselected}),
            })
            .await;
        assert!(!denied.success);
        Ok(RunReport {
            run_id: r.run_id,
            outcome: RunOutcome::Completed,
            final_text: "Saved portfolio read successfully".into(),
        })
    }
    async fn close(&mut self) -> lugus_agent::Result<()> {
        Ok(())
    }
}
#[tokio::test]
async fn portfolio_chat_uses_only_selected_frozen_evidence_without_research() {
    let h = support::Harness::new(&[], HostBounds::default(), Limits::default()).await;
    let c = h.app.create_conversation("c", "Portfolio").await.unwrap();
    let p=h.app.execute_portfolio(serde_json::from_value(json!({"request_id":"p","portfolio_id":null,"expected_revision":"0","mutation":{"kind":"create_portfolio","name":"Original"}})).unwrap()).await.unwrap().portfolio_id;
    let request = SnapshotRequest {
        request_id: "selected".into(),
        portfolio_id: p.clone(),
        account_id: None,
        expected_revision: 1,
        conversation_id: c.id.clone(),
    };
    let selected = h
        .app
        .create_portfolio_snapshot(request.clone())
        .await
        .unwrap();
    let other = h
        .app
        .create_portfolio_snapshot(SnapshotRequest {
            request_id: "other".into(),
            ..request
        })
        .await
        .unwrap();
    h.app.execute_portfolio(serde_json::from_value(json!({"request_id":"rename","portfolio_id":p,"expected_revision":"1","mutation":{"kind":"rename_portfolio","name":"Current"}})).unwrap()).await.unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let host = ConversationHost::start_with_interpreter(
        h.app.clone(),
        Arc::new(Reader {
            selected: selected.id.clone(),
            unselected: other.id,
        }),
        ConversationOptions::default(),
        Arc::new(NoResearch(count.clone())),
    )
    .await
    .unwrap();
    let run = host
        .send(SendMessageRequest {
            research_brief: None,
            company_hint: None,
            conversation_id: c.id.clone(),
            request_id: "question".into(),
            text: "What do I own?".into(),
            selected: vec![SelectedReference::Portfolio { id: selected.id }],
        })
        .await
        .unwrap();
    let terminal = host.wait(&c.id, &run.id).await.unwrap();
    assert_eq!(
        terminal.status,
        RunStatus::Completed,
        "{:?}",
        terminal.error
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    host.shutdown().await.unwrap();
}
