use lugus_agent::{
    AgentRuntime, RunOutcome, RunReport, RunRequest, RuntimeEvent, ToolCall, ToolExecutor,
};
use lugus_app::{
    ErrorKind,
    conversations::{ContextSnapshot, RunRecord, RunStatus, RuntimeFactory},
    research::{Interpreter, RuntimeInterpreter, Workflow},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, watch};

#[derive(Clone, Copy)]
enum Mode {
    Good,
    BadIdentity,
    InvalidJson,
    Cancelled,
    Hang,
    Panic,
    Flood,
    CloseHang,
    CreateHang,
    Authentication,
    NeedsAttention,
}
struct Factory {
    mode: Mode,
    closed: Arc<AtomicUsize>,
    created: Arc<AtomicUsize>,
}
struct Runtime {
    mode: Mode,
    closed: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl RuntimeFactory for Factory {
    async fn create(&self) -> lugus_app::Result<Box<dyn AgentRuntime>> {
        self.created.fetch_add(1, Ordering::SeqCst);
        if matches!(self.mode, Mode::CreateHang) {
            std::future::pending::<()>().await;
        }
        Ok(Box::new(Runtime {
            mode: self.mode,
            closed: self.closed.clone(),
        }))
    }
}
#[async_trait::async_trait]
impl AgentRuntime for Runtime {
    async fn run(
        &mut self,
        request: RunRequest,
        tools: &dyn ToolExecutor,
        events: mpsc::Sender<RuntimeEvent>,
        _cancel: watch::Receiver<bool>,
    ) -> lugus_agent::Result<RunReport> {
        assert!(request.tools.is_empty());
        assert!(!request.allow_web_search);
        assert_eq!(request.limits.max_tool_calls, 1);
        lugus_agent::validate_request(&request).unwrap();
        assert!(request.context.contains("2026-09-10"));
        assert!(request.prompt.contains("hello"));
        assert!(
            !tools
                .execute(ToolCall {
                    run_id: request.run_id.clone(),
                    call_id: "attempt".into(),
                    name: "financial.search".into(),
                    arguments: serde_json::json!({})
                })
                .await
                .success
        );
        match self.mode {
            Mode::Authentication => return Err(lugus_agent::Error::AuthenticationRequired),
            Mode::NeedsAttention => {
                return Err(lugus_agent::Error::NeedsAttention(
                    "private credentials".into(),
                ));
            }
            Mode::Hang => std::future::pending::<()>().await,
            Mode::Panic => panic!("private runtime failure"),
            Mode::Flood => {
                for _ in 0..5000 {
                    if events
                        .send(RuntimeEvent::TextDelta {
                            text: "private thought".into(),
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                std::future::pending::<()>().await;
            }
            _ => {}
        }
        // More events than channel capacity must be consumed without exposing them.
        for _ in 0..32 {
            events
                .send(RuntimeEvent::TextDelta {
                    text: "private thought".into(),
                })
                .await
                .unwrap();
        }
        Ok(RunReport {
            run_id: if matches!(self.mode, Mode::BadIdentity) {
                "other".into()
            } else {
                request.run_id
            },
            outcome: if matches!(self.mode, Mode::Cancelled) {
                RunOutcome::Cancelled
            } else {
                RunOutcome::Completed
            },
            final_text: if matches!(self.mode, Mode::InvalidJson) {
                "not JSON".into()
            } else {
                r#"{"workflow":"conversation","subjects":[],"start":null,"end":null,"clarification":null}"#.into()
            },
        })
    }
    async fn close(&mut self) -> lugus_agent::Result<()> {
        self.closed.fetch_add(1, Ordering::SeqCst);
        if matches!(self.mode, Mode::CloseHang) {
            std::future::pending::<()>().await;
        }
        Ok(())
    }
}
fn run() -> RunRecord {
    RunRecord {
        company_hint: None,
        id: "run".into(),
        conversation_id: "conversation".into(),
        workspace_id: "workspace".into(),
        request_id: "request".into(),
        user_message_id: "message".into(),
        epoch: 1,
        status: RunStatus::Running,
        input: ContextSnapshot {
            policy: "test".into(),
            serialized: "hello".into(),
            message_ids: vec![],
            references: vec![],
            omitted_messages: 0,
        },
        created_at: "2026-09-10T12:00:00Z".parse().unwrap(),
        finished_at: None,
        error: None,
    }
}
fn interpreter(mode: Mode) -> (RuntimeInterpreter, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let closed = Arc::new(AtomicUsize::new(0));
    let created = Arc::new(AtomicUsize::new(0));
    let factory = Arc::new(Factory {
        mode,
        closed: closed.clone(),
        created: created.clone(),
    });
    (
        RuntimeInterpreter::new(
            factory,
            Duration::from_millis(500),
            Duration::from_millis(10),
        ),
        closed,
        created,
    )
}
#[tokio::test]
async fn isolated_interpretation_drains_events_and_closes() {
    let (interpreter, closed, created) = interpreter(Mode::Good);
    let (_sender, cancel) = watch::channel(false);
    assert_eq!(
        interpreter
            .interpret(&run(), Some("{}"), cancel)
            .await
            .unwrap()
            .workflow,
        Workflow::Conversation
    );
    assert_eq!(created.load(Ordering::SeqCst), 1);
    assert_eq!(closed.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn all_failure_paths_close_exactly_once() {
    for (mode, kind) in [
        (Mode::BadIdentity, ErrorKind::InvalidInput),
        (Mode::InvalidJson, ErrorKind::InvalidInput),
        (Mode::Cancelled, ErrorKind::Cancelled),
        (Mode::Hang, ErrorKind::Timeout),
        (Mode::Panic, ErrorKind::Unavailable),
        (Mode::Flood, ErrorKind::ResourceLimit),
        (Mode::CloseHang, ErrorKind::Timeout),
        (Mode::Authentication, ErrorKind::AuthenticationRequired),
        (Mode::NeedsAttention, ErrorKind::NeedsAttention),
    ] {
        let (interpreter, closed, _) = interpreter(mode);
        let (_sender, cancel) = watch::channel(false);
        let error = interpreter
            .interpret(&run(), None, cancel)
            .await
            .unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(closed.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn creation_deadline_and_pre_cancel_do_not_leak_runtime() {
    let (interpreter, closed, created) = interpreter(Mode::CreateHang);
    let (sender, cancel) = watch::channel(false);
    assert_eq!(
        interpreter
            .interpret(&run(), None, cancel.clone())
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Timeout
    );
    assert_eq!(closed.load(Ordering::SeqCst), 0);
    sender.send(true).unwrap();
    assert_eq!(
        interpreter
            .interpret(&run(), None, cancel)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(created.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn active_cancellation_closes_and_invalid_context_never_creates() {
    let (interpreter, closed, created) = interpreter(Mode::Hang);
    let (sender, cancel) = watch::channel(false);
    let signal = async {
        tokio::time::sleep(Duration::from_millis(5)).await;
        sender.send(true).unwrap();
    };
    let record = run();
    let (result, _) = tokio::join!(interpreter.interpret(&record, None, cancel), signal);
    assert_eq!(result.unwrap_err().kind, ErrorKind::Cancelled);
    assert_eq!(closed.load(Ordering::SeqCst), 1);
    let (_sender, cancel) = watch::channel(false);
    assert!(
        interpreter
            .interpret(&run(), Some("not json"), cancel)
            .await
            .is_err()
    );
    assert_eq!(created.load(Ordering::SeqCst), 1);
}
