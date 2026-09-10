mod support;
use lugus_agent::{
    AgentRuntime, RunOutcome, RunReport, RunRequest, RunSubject, RuntimeEvent, ToolExecutor,
};
use lugus_app::{conversations::*, *};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{mpsc, watch};

#[derive(Clone, Default)]
struct Factory {
    calls: Arc<AtomicUsize>,
    closes: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<RunRequest>>>,
    hold: bool,
}
struct Runtime {
    factory: Arc<Factory>,
}
#[async_trait::async_trait]
impl RuntimeFactory for Factory {
    async fn create(&self) -> Result<Box<dyn AgentRuntime>> {
        Ok(Box::new(Runtime {
            factory: Arc::new(self.clone()),
        }))
    }
}
#[async_trait::async_trait]
impl AgentRuntime for Runtime {
    async fn run(
        &mut self,
        request: RunRequest,
        _: &dyn ToolExecutor,
        events: mpsc::Sender<RuntimeEvent>,
        _: watch::Receiver<bool>,
    ) -> lugus_agent::Result<RunReport> {
        self.factory.calls.fetch_add(1, Ordering::SeqCst);
        self.factory.requests.lock().unwrap().push(request.clone());
        events
            .send(RuntimeEvent::TextDelta {
                text: "partial".into(),
            })
            .await
            .unwrap();
        if self.factory.hold {
            std::future::pending::<()>().await;
        }
        Ok(RunReport {
            run_id: request.run_id,
            outcome: RunOutcome::Completed,
            final_text: "Answer".into(),
        })
    }
    async fn close(&mut self) -> lugus_agent::Result<()> {
        self.factory.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
fn send(c: &Conversation, id: &str) -> SendMessageRequest {
    SendMessageRequest {
        conversation_id: c.id.clone(),
        request_id: id.into(),
        text: "Research two companies".into(),
        selected: vec![],
    }
}
fn page() -> PageRequest {
    PageRequest {
        offset: 0,
        limit: 10,
    }
}
async fn host(hold: bool) -> (support::Harness, ConversationHost, Arc<Factory>) {
    let h = support::Harness::new(&[], HostBounds::default(), Limits::default()).await;
    let factory = Arc::new(Factory {
        hold,
        ..Default::default()
    });
    let host = ConversationHost::start(
        h.app.clone(),
        factory.clone(),
        ConversationOptions::default(),
    )
    .await
    .unwrap();
    (h, host, factory)
}
#[tokio::test]
async fn thesis_free_two_turns_freeze_history_and_reopen_without_replay() {
    let (h, host, factory) = host(false).await;
    let c = host.create("create", "Research").await.unwrap();
    let first = host.send(send(&c, "one")).await.unwrap();
    assert_eq!(
        host.wait(&c.id, &first.id).await.unwrap().status,
        RunStatus::Completed
    );
    let second = host.send(send(&c, "two")).await.unwrap();
    assert_eq!(
        host.wait(&c.id, &second.id).await.unwrap().status,
        RunStatus::Completed
    );
    let requests = factory.requests.lock().unwrap().clone();
    assert!(matches!(
        requests[0].subject,
        RunSubject::Conversation { .. }
    ));
    assert!(requests[0].context.is_empty());
    assert!(!requests[0].allow_web_search);
    assert!(!requests[0].instructions.contains("Research two companies"));
    assert!(requests[1].prompt.contains("Answer"));
    assert_eq!(second.input.message_ids.len(), 3);
    assert_eq!(host.messages(&c.id, page()).await.unwrap().items.len(), 4);
    host.cancel(&c.id, &first.id).await.unwrap();
    assert_eq!(
        host.status(&c.id, &first.id).await.unwrap().status,
        RunStatus::Completed
    );
    assert_eq!(
        host.activity(&c.id, &first.id, page())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    host.shutdown().await.unwrap();
    let reopened = ConversationHost::start(
        h.app.clone(),
        factory.clone(),
        ConversationOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(reopened.send(send(&c, "one")).await.unwrap().id, first.id);
    assert_eq!(factory.calls.load(Ordering::SeqCst), 2);
    reopened.shutdown().await.unwrap();
}
#[tokio::test]
async fn bounded_independent_runs_duplicate_receipt_and_conflict_have_no_extra_messages() {
    let (_h, host, factory) = host(true).await;
    let mut conversations = Vec::new();
    let mut receipts = Vec::new();
    for n in 0..5 {
        conversations.push(host.create(&format!("c{n}"), "Research").await.unwrap());
    }
    for c in &conversations[..4] {
        receipts.push(host.send(send(c, "one")).await.unwrap());
    }
    assert_eq!(
        host.send(send(&conversations[0], "one")).await.unwrap().id,
        receipts[0].id
    );
    assert_eq!(
        host.send(send(&conversations[4], "one"))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    assert!(
        host.messages(&conversations[4].id, page())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        host.runs(&conversations[4].id, page())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    host.cancel(&conversations[0].id, &receipts[0].id)
        .await
        .unwrap();
    host.wait(&conversations[0].id, &receipts[0].id)
        .await
        .unwrap();
    assert_eq!(
        host.send(send(&conversations[1], "two"))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    host.shutdown().await.unwrap();
    assert!(factory.calls.load(Ordering::SeqCst) <= 4);
    assert_eq!(
        factory.closes.load(Ordering::SeqCst),
        factory.calls.load(Ordering::SeqCst)
    );
}
#[tokio::test]
async fn concurrent_identical_sends_launch_once_and_shutdown_survives_dropped_waiter() {
    let (_h, host, factory) = host(true).await;
    let c = host.create("create", "Research").await.unwrap();
    let (a, b) = tokio::join!(host.send(send(&c, "one")), host.send(send(&c, "one")));
    assert_eq!(a.unwrap().id, b.unwrap().id);
    let mut shutdown = Box::pin(host.shutdown());
    tokio::select! { _ = &mut shutdown => {}, _ = tokio::task::yield_now() => {} }
    drop(shutdown);
    host.shutdown().await.unwrap();
    assert!(factory.calls.load(Ordering::SeqCst) <= 1);
    assert_eq!(
        host.runs(&c.id, page()).await.unwrap().items[0].status,
        RunStatus::Interrupted
    );
}

#[derive(Clone)]
struct Behavior {
    mode: &'static str,
    closes: Arc<AtomicUsize>,
    results: Arc<Mutex<Vec<lugus_agent::ToolResult>>>,
    late_events: Arc<Mutex<Option<mpsc::Sender<RuntimeEvent>>>>,
    close_started: watch::Sender<bool>,
    close_release: watch::Receiver<bool>,
}
impl Behavior {
    fn new(mode: &'static str) -> (Self, watch::Receiver<bool>, watch::Sender<bool>) {
        let (close_started, started) = watch::channel(false);
        let (release, close_release) = watch::channel(false);
        (
            Self {
                mode,
                closes: Arc::new(AtomicUsize::new(0)),
                results: Arc::new(Mutex::new(vec![])),
                late_events: Arc::new(Mutex::new(None)),
                close_started,
                close_release,
            },
            started,
            release,
        )
    }
}
#[async_trait::async_trait]
impl RuntimeFactory for Behavior {
    async fn create(&self) -> Result<Box<dyn AgentRuntime>> {
        match self.mode {
            "factory_error" => Err(AppError::new(
                ErrorKind::Unavailable,
                "factory failed",
                false,
            )),
            "factory_panic" => panic!("factory fixture"),
            "factory_hold" => std::future::pending().await,
            _ => Ok(Box::new(self.clone())),
        }
    }
}
#[async_trait::async_trait]
impl AgentRuntime for Behavior {
    async fn run(
        &mut self,
        request: RunRequest,
        tools: &dyn ToolExecutor,
        events: mpsc::Sender<RuntimeEvent>,
        _: watch::Receiver<bool>,
    ) -> lugus_agent::Result<RunReport> {
        match self.mode {
            "panic" => panic!("runtime fixture"),
            "retain_events" => {
                *self.late_events.lock().unwrap() = Some(events.clone());
            }
            "hold" => std::future::pending::<()>().await,
            "flood" => {
                for _ in 0..2000 {
                    if events
                        .send(RuntimeEvent::TextDelta {
                            text: "partial".into(),
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
            "large_event" => {
                let _ = events
                    .send(RuntimeEvent::TextDelta {
                        text: "x".repeat(2 * 1024 * 1024),
                    })
                    .await;
                std::future::pending::<()>().await;
            }
            "tool_duplicate" | "tool_mismatch" | "tool_bad_run" | "tool_two" => {
                let call = lugus_agent::ToolCall {
                    run_id: if self.mode == "tool_bad_run" {
                        "foreign".into()
                    } else {
                        request.run_id.clone()
                    },
                    call_id: "call-one".into(),
                    name: "lugus_fetch_filings".into(),
                    arguments: serde_json::json!({"instance_id":"one", "query": match support::command("one") { FetchCommand::Filings { query, .. } => query, _ => unreachable!() }}),
                };
                let result = tools.execute(call.clone()).await;
                self.results.lock().unwrap().push(result);
                let mut second = call;
                if self.mode == "tool_mismatch" {
                    second.arguments["instance_id"] = "changed".into();
                }
                if self.mode == "tool_two" {
                    second.call_id = "call-two".into();
                }
                let result = tools.execute(second).await;
                self.results.lock().unwrap().push(result);
            }
            "tool_pending" | "tool_abandon" => {
                let call = lugus_agent::ToolCall {
                    run_id: request.run_id.clone(),
                    call_id: "call-one".into(),
                    name: "lugus_fetch_filings".into(),
                    arguments: serde_json::json!({"instance_id":"one", "query": match support::command("one") { FetchCommand::Filings { query, .. } => query, _ => unreachable!() }}),
                };
                if self.mode == "tool_pending" {
                    let _ = tokio::join!(tools.execute(call.clone()), tools.execute(call));
                } else {
                    let mut pending = Box::pin(tools.execute(call));
                    tokio::select! { biased; _ = &mut pending => {}, _ = tokio::task::yield_now() => {} }
                }
            }
            "tool_drop" => {
                let call = lugus_agent::ToolCall {
                    run_id: request.run_id.clone(),
                    call_id: "call-one".into(),
                    name: "lugus_fetch_filings".into(),
                    arguments: serde_json::json!({"instance_id":"one", "query": match support::command("one") { FetchCommand::Filings { query, .. } => query, _ => unreachable!() }}),
                };
                // Cancellation drops this runtime future while the owned journal call is waiting on the provider.
                let _ = tools.execute(call).await;
            }
            _ => {}
        }
        Ok(RunReport {
            run_id: if self.mode == "wrong_id" {
                "other".into()
            } else {
                request.run_id
            },
            outcome: RunOutcome::Completed,
            final_text: match self.mode {
                "oversized" => "x".repeat(40000),
                "empty" => String::new(),
                _ => "Answer".into(),
            },
        })
    }
    async fn close(&mut self) -> lugus_agent::Result<()> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        self.close_started.send_replace(true);
        if self.mode == "close_hold" {
            std::future::pending::<()>().await;
        }
        if matches!(self.mode, "close_gate" | "close_gate_error") {
            while !*self.close_release.borrow_and_update() {
                self.close_release.changed().await.unwrap();
            }
        }
        if matches!(self.mode, "close_error" | "close_gate_error") {
            return Err(lugus_agent::Error::Process("private diagnostic".into()));
        }
        Ok(())
    }
}
async fn terminal(host: &ConversationHost, c: &Conversation, run: &RunRecord) -> RunRecord {
    tokio::time::timeout(std::time::Duration::from_secs(3), host.wait(&c.id, &run.id))
        .await
        .expect("finite supervision")
        .unwrap()
}
#[tokio::test]
async fn runtime_failures_are_terminal_and_cleanup_is_finite() {
    for mode in [
        "factory_error",
        "factory_panic",
        "factory_hold",
        "panic",
        "hold",
        "wrong_id",
        "oversized",
        "empty",
        "flood",
        "large_event",
        "close_error",
    ] {
        let h = support::Harness::new(&[], HostBounds::default(), Limits::default()).await;
        let (behavior, _, _) = Behavior::new(mode);
        let options = ConversationOptions {
            run_limits: lugus_agent::RunLimits {
                timeout: std::time::Duration::from_millis(80),
                ..default_conversation_run_limits()
            },
            ..Default::default()
        };
        let host = ConversationHost::start(h.app.clone(), Arc::new(behavior.clone()), options)
            .await
            .unwrap();
        let c = host.create("create", "Research").await.unwrap();
        let run = host.send(send(&c, "one")).await.unwrap();
        let completed = terminal(&host, &c, &run).await;
        assert_eq!(completed.status, RunStatus::Failed, "{mode}: {completed:?}");
        assert_eq!(
            host.messages(&c.id, page()).await.unwrap().items.len(),
            1,
            "{mode}"
        );
        assert_eq!(
            behavior.closes.load(Ordering::SeqCst),
            usize::from(!mode.starts_with("factory")),
            "{mode}"
        );
        assert!(
            !completed
                .error
                .unwrap()
                .message
                .contains("private diagnostic")
        );
        let shutdown = host.shutdown().await;
        if mode == "close_error" {
            assert!(shutdown.is_err());
            assert_eq!(host.shutdown().await, shutdown);
        } else {
            shutdown.unwrap();
        }
    }
}
#[tokio::test]
async fn exact_duplicate_tool_result_has_one_durable_call_and_one_effect() {
    let h = support::Harness::new(&[("one", "ok")], HostBounds::default(), Limits::default()).await;
    let (behavior, _, _) = Behavior::new("tool_duplicate");
    let host = ConversationHost::start(
        h.app.clone(),
        Arc::new(behavior.clone()),
        ConversationOptions::default(),
    )
    .await
    .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    let run = host.send(send(&c, "one")).await.unwrap();
    assert_eq!(terminal(&host, &c, &run).await.status, RunStatus::Completed);
    let records = host
        .tool_records(&c.id, &run.id, page())
        .await
        .unwrap()
        .items;
    assert_eq!(records.len(), 1);
    let results = behavior.results.lock().unwrap().clone();
    assert_eq!(results[0], results[1]);
    assert_eq!(
        records[0].outcome,
        Some(ToolOutcome::Returned {
            result: results[0].clone()
        })
    );
    let job: JobStatus = serde_json::from_str(&results[0].content).unwrap();
    assert_eq!(job.state, JobState::Succeeded);
    let scope = Scope {
        workspace_id: c.workspace_id.clone(),
        request_id: "inspect".into(),
        run_id: None,
    };
    let fetch = h
        .app
        .read_fetch(&scope, job.fetch_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(fetch.runs.len(), 1);
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn requested_tool_call_limit_prevents_second_effect() {
    let h = support::Harness::new(&[("one", "ok")], HostBounds::default(), Limits::default()).await;
    let (behavior, _, _) = Behavior::new("tool_two");
    let host = ConversationHost::start(
        h.app.clone(),
        Arc::new(behavior),
        ConversationOptions {
            run_limits: lugus_agent::RunLimits {
                max_tool_calls: 1,
                ..default_conversation_run_limits()
            },
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    let run = host.send(send(&c, "one")).await.unwrap();
    assert_eq!(terminal(&host, &c, &run).await.status, RunStatus::Failed);
    assert_eq!(
        host.tool_records(&c.id, &run.id, page())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancellation_joins_partial_provider_effect_and_preserves_exact_receipt() {
    let h = support::Harness::new(
        &[("one", "second_blocked")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let (behavior, _, _) = Behavior::new("tool_drop");
    let host = ConversationHost::start(
        h.app.clone(),
        Arc::new(behavior),
        ConversationOptions::default(),
    )
    .await
    .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    let run = host.send(send(&c, "one")).await.unwrap();
    h.barrier("one", "second").await;
    let waiter = tokio::spawn({
        let host = host.clone();
        let c = c.clone();
        let run = run.clone();
        async move { host.wait(&c.id, &run.id).await }
    });
    waiter.abort();
    assert_eq!(
        host.status(&c.id, &run.id).await.unwrap().status,
        RunStatus::Running
    );
    host.cancel(&c.id, &run.id).await.unwrap();
    assert_eq!(
        terminal(&host, &c, &run).await.status,
        RunStatus::Interrupted
    );
    let records = host
        .tool_records(&c.id, &run.id, page())
        .await
        .unwrap()
        .items;
    let Some(ToolOutcome::Returned { result }) = &records[0].outcome else {
        panic!("exact partial receipt required")
    };
    assert!(!result.success);
    let job: JobStatus = serde_json::from_str(&result.content).unwrap();
    assert_eq!(job.state, JobState::Cancelled);
    let scope = Scope {
        workspace_id: c.workspace_id,
        request_id: "inspect".into(),
        run_id: None,
    };
    let fetch = h
        .app
        .read_fetch(&scope, job.fetch_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(fetch.runs.len(), 1);
    host.shutdown().await.unwrap();
    assert!(LocalExecutionLease::acquire(&h.application).is_ok());
}
#[tokio::test]
async fn cancelled_shutdown_retains_ownership_until_runtime_close_and_late_cancel_cannot_overwrite_completion()
 {
    let h = support::Harness::new(&[], HostBounds::default(), Limits::default()).await;
    let (behavior, mut closing, release) = Behavior::new("close_gate");
    let host = ConversationHost::start(
        h.app.clone(),
        Arc::new(behavior),
        ConversationOptions::default(),
    )
    .await
    .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    let run = host.send(send(&c, "one")).await.unwrap();
    while !*closing.borrow_and_update() {
        closing.changed().await.unwrap();
    }
    assert!(LocalExecutionLease::acquire(&h.application).is_err());
    let shutdown = tokio::spawn({
        let host = host.clone();
        async move { host.shutdown().await }
    });
    tokio::task::yield_now().await;
    shutdown.abort();
    assert!(LocalExecutionLease::acquire(&h.application).is_err());
    release.send_replace(true);
    host.shutdown().await.unwrap();
    assert_eq!(
        host.status(&c.id, &run.id).await.unwrap().status,
        RunStatus::Interrupted
    );
    assert!(LocalExecutionLease::acquire(&h.application).is_ok());
}

async fn custom_app(
    limits: ConversationLimits,
    clock: Box<dyn Clock>,
) -> (tempfile::TempDir, Application, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let financial = root.path().join("financial.sqlite");
    let path = root.path().join("application.sqlite");
    let repositories = Arc::new(SqliteRepositoryFactory::new(&financial));
    repositories.initialize().unwrap();
    let store = SqliteApplicationStore::open_with_conversation_limits(
        &path,
        Box::new(lugus_financial::storage::SqliteRepository::open(&financial).unwrap()),
        Limits::default(),
        limits,
        clock,
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap();
    let app = Application::start(
        vec![],
        repositories,
        Box::new(store),
        Limits::default(),
        HostBounds::default(),
        Box::new(RandomIds::new().unwrap()),
    )
    .await
    .unwrap();
    (root, app, path)
}
#[tokio::test]
async fn uncooperative_close_has_bounded_failure_and_releases_lease_after_shutdown() {
    let (_root, app, path) = custom_app(
        ConversationLimits {
            runtime_close_timeout_ms: 20,
            ..Default::default()
        },
        Box::new(SystemClock),
    )
    .await;
    let (behavior, _, _) = Behavior::new("close_hold");
    let host = ConversationHost::start(app, Arc::new(behavior), ConversationOptions::default())
        .await
        .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    let run = host.send(send(&c, "one")).await.unwrap();
    let record = terminal(&host, &c, &run).await;
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(record.error.unwrap().kind, ErrorKind::Timeout);
    let first = host.shutdown().await.unwrap_err();
    assert_eq!(first.message, "runtime cleanup timed out");
    assert_eq!(host.shutdown().await.unwrap_err(), first);
    assert!(LocalExecutionLease::acquire(path).is_ok());
}
#[tokio::test]
async fn tool_identity_and_changed_duplicate_stop_turn_without_extra_effects() {
    for mode in ["tool_bad_run", "tool_mismatch"] {
        let h =
            support::Harness::new(&[("one", "ok")], HostBounds::default(), Limits::default()).await;
        let (behavior, _, _) = Behavior::new(mode);
        let host = ConversationHost::start(
            h.app.clone(),
            Arc::new(behavior),
            ConversationOptions::default(),
        )
        .await
        .unwrap();
        let c = host.create("create", "Research").await.unwrap();
        let run = host.send(send(&c, "one")).await.unwrap();
        assert_eq!(terminal(&host, &c, &run).await.status, RunStatus::Failed);
        assert_eq!(
            host.tool_records(&c.id, &run.id, page())
                .await
                .unwrap()
                .items
                .len(),
            usize::from(mode == "tool_mismatch")
        );
        host.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn small_event_channel_and_activity_storage_failure_cannot_block_finalization() {
    let (_root, app, path) = custom_app(
        ConversationLimits {
            event_capacity: 1,
            ..Default::default()
        },
        Box::new(SystemClock),
    )
    .await;
    let (behavior, _, _) = Behavior::new("flood");
    let host = ConversationHost::start(
        app.clone(),
        Arc::new(behavior),
        ConversationOptions::default(),
    )
    .await
    .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    let connection = rusqlite::Connection::open(path).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_activity BEFORE INSERT ON conversation_activity BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    let run = host.send(send(&c, "one")).await.unwrap();
    assert_eq!(terminal(&host, &c, &run).await.status, RunStatus::Failed);
    assert!(
        host.activity(&c.id, &run.id, page())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(host.messages(&c.id, page()).await.unwrap().items.len(), 1);
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn ordinary_reads_do_not_recover_and_recovery_host_does_not_replay_unknown_tools() {
    let h = support::Harness::new(&[], HostBounds::default(), Limits::default()).await;
    let c = h
        .app
        .create_conversation("create", "Research")
        .await
        .unwrap();
    let mut store = SqliteApplicationStore::open(
        &h.application,
        Box::new(lugus_financial::storage::SqliteRepository::open(&h.financial).unwrap()),
        Limits::default(),
        Box::new(SystemClock),
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap();
    let lease = LocalExecutionLease::acquire(&h.application).unwrap();
    let epoch = store.activate(&lease).unwrap();
    let run = store.admit(&epoch, &send(&c, "one")).unwrap();
    let attempt = store.start(&epoch, &c.id, &run.id).unwrap();
    store
        .begin_tool(
            &attempt,
            &ToolIntent {
                call_id: "unknown".into(),
                name: "lugus_fetch_filings".into(),
                arguments: "{}".into(),
                result_capacity: 512,
            },
        )
        .unwrap();
    assert_eq!(
        h.app.conversation_run(&c.id, &run.id).await.unwrap().status,
        RunStatus::Running
    );
    assert_eq!(
        ConversationHost::recover(h.app.clone())
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::Conflict
    );
    drop(attempt);
    drop(epoch);
    drop(lease);
    let host = ConversationHost::recover(h.app.clone()).await.unwrap();
    assert_eq!(
        host.status(&c.id, &run.id).await.unwrap().status,
        RunStatus::Interrupted
    );
    assert_eq!(host.send(send(&c, "one")).await.unwrap().id, run.id);
    assert!(
        host.tool_records(&c.id, &run.id, page())
            .await
            .unwrap()
            .items[0]
            .outcome
            .is_none()
    );
    host.shutdown().await.unwrap();
}
struct GateClock {
    armed: Arc<std::sync::atomic::AtomicBool>,
    entered: watch::Sender<bool>,
    release: Arc<(Mutex<bool>, std::sync::Condvar)>,
}
impl Clock for GateClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.send_replace(true);
            let (mutex, condition) = &*self.release;
            let mut released = mutex.lock().unwrap();
            while !*released {
                released = condition.wait(released).unwrap();
            }
        }
        chrono::Utc::now()
    }
}
#[tokio::test]
async fn dropped_send_during_admission_is_registered_and_shutdown_joins_it() {
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (entered, mut admission) = watch::channel(false);
    let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let (_root, app, path) = custom_app(
        ConversationLimits::default(),
        Box::new(GateClock {
            armed: armed.clone(),
            entered,
            release: release.clone(),
        }),
    )
    .await;
    let factory = Arc::new(Factory::default());
    let host = ConversationHost::start(app, factory.clone(), ConversationOptions::default())
        .await
        .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    armed.store(true, Ordering::SeqCst);
    let sender = tokio::spawn({
        let host = host.clone();
        let request = send(&c, "one");
        async move { host.send(request).await }
    });
    while !*admission.borrow_and_update() {
        admission.changed().await.unwrap();
    }
    sender.abort();
    let shutdown = tokio::spawn({
        let host = host.clone();
        async move { host.shutdown().await }
    });
    tokio::task::yield_now().await;
    assert!(LocalExecutionLease::acquire(&path).is_err());
    *release.0.lock().unwrap() = true;
    release.1.notify_all();
    shutdown.await.unwrap().unwrap();
    let runs = host.runs(&c.id, page()).await.unwrap().items;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, RunStatus::Interrupted);
    assert_eq!(factory.calls.load(Ordering::SeqCst), 0);
    assert!(LocalExecutionLease::acquire(path).is_ok());
}

#[tokio::test]
async fn journal_budget_failure_stops_turn_and_records_safe_failure() {
    let root = tempfile::tempdir().unwrap();
    let financial = root.path().join("financial.sqlite");
    let path = root.path().join("application.sqlite");
    let repositories = Arc::new(SqliteRepositoryFactory::new(&financial));
    repositories.initialize().unwrap();
    let store = SqliteApplicationStore::open_with_conversation_limits(
        &path,
        Box::new(lugus_financial::storage::SqliteRepository::open(&financial).unwrap()),
        Limits::default(),
        ConversationLimits {
            tool_record_bytes: 17024,
            ..Default::default()
        },
        Box::new(SystemClock),
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap();
    let providers = vec![ConfiguredProvider {
        active: true,
        factory: Arc::new(ProcessProviderFactory {
            manifest: lugus_financial::plugin::Manifest {
                id: "worker-fixture".into(),
                version: "1".into(),
                protocol_version: 1,
                command: "python3".into(),
                args: vec![format!(
                    "{}/tests/fixtures/worker.py",
                    env!("CARGO_MANIFEST_DIR")
                )],
            },
            directory: root.path().into(),
            instance_id: "one".into(),
            config: serde_json::json!({"mode":"ok","barrier":root.path().join("one")}),
        }),
    }];
    let app = Application::start(
        providers,
        repositories,
        Box::new(store),
        Limits::default(),
        HostBounds::default(),
        Box::new(RandomIds::new().unwrap()),
    )
    .await
    .unwrap();
    let (behavior, _, _) = Behavior::new("tool_duplicate");
    let host = ConversationHost::start(app, Arc::new(behavior), ConversationOptions::default())
        .await
        .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    let run = host.send(send(&c, "one")).await.unwrap();
    assert_eq!(terminal(&host, &c, &run).await.status, RunStatus::Failed);
    let calls = host
        .tool_records(&c.id, &run.id, page())
        .await
        .unwrap()
        .items;
    assert_eq!(calls.len(), 1);
    assert!(
        matches!(&calls[0].outcome,Some(ToolOutcome::Failed {error}) if error.kind==ErrorKind::ResourceLimit)
    );
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn concurrent_pending_duplicate_and_abandoned_tool_future_never_redispatch() {
    for mode in ["tool_pending", "tool_abandon"] {
        let h = support::Harness::new(
            &[("one", "blocked")],
            HostBounds::default(),
            Limits::default(),
        )
        .await;
        let (behavior, _, _) = Behavior::new(mode);
        let host = ConversationHost::start(
            h.app.clone(),
            Arc::new(behavior),
            ConversationOptions::default(),
        )
        .await
        .unwrap();
        let c = host.create("create", "Research").await.unwrap();
        let run = host.send(send(&c, "one")).await.unwrap();
        assert_eq!(
            terminal(&host, &c, &run).await.status,
            RunStatus::Failed,
            "{mode}"
        );
        assert_eq!(
            host.tool_records(&c.id, &run.id, page())
                .await
                .unwrap()
                .items
                .len(),
            1
        );
        host.shutdown().await.unwrap();
        assert!(LocalExecutionLease::acquire(&h.application).is_ok());
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_waiters_always_observe_committed_terminal_state() {
    let (_h, host, _) = host(false).await;
    let c = host.create("create", "Research").await.unwrap();
    for n in 0..20 {
        let run = host.send(send(&c, &format!("turn-{n}"))).await.unwrap();
        let mut waiters = tokio::task::JoinSet::new();
        for _ in 0..16 {
            let host = host.clone();
            let c = c.clone();
            let run = run.clone();
            waiters.spawn(async move { host.wait(&c.id, &run.id).await });
        }
        while let Some(result) = waiters.join_next().await {
            assert_eq!(result.unwrap().unwrap().status, RunStatus::Completed);
        }
    }
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn tool_result_storage_failure_keeps_unknown_intent_and_fails_turn() {
    let h = support::Harness::new(&[("one", "ok")], HostBounds::default(), Limits::default()).await;
    let (behavior, _, _) = Behavior::new("tool_duplicate");
    let host = ConversationHost::start(
        h.app.clone(),
        Arc::new(behavior),
        ConversationOptions::default(),
    )
    .await
    .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    let connection = rusqlite::Connection::open(&h.application).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_tool_finish BEFORE UPDATE ON conversation_tools BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    let run = host.send(send(&c, "one")).await.unwrap();
    assert_eq!(terminal(&host, &c, &run).await.status, RunStatus::Failed);
    let calls = host
        .tool_records(&c.id, &run.id, page())
        .await
        .unwrap()
        .items;
    assert_eq!(calls.len(), 1);
    assert!(calls[0].outcome.is_none());
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn selected_evidence_is_frozen_in_second_turn_user_data_after_workspace_changes() {
    let h = support::Harness::new(&[("one", "ok")], HostBounds::default(), Limits::default()).await;
    let factory = Arc::new(Factory::default());
    let host = ConversationHost::start(
        h.app.clone(),
        factory.clone(),
        ConversationOptions::default(),
    )
    .await
    .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    let first = host.send(send(&c, "one")).await.unwrap();
    terminal(&host, &c, &first).await;
    let scope = Scope {
        workspace_id: c.workspace_id.clone(),
        request_id: "evidence".into(),
        run_id: None,
    };
    let receipt = h
        .app
        .submit_manual(&scope, support::command("one"))
        .unwrap();
    let job = h.app.wait(&scope, &receipt.id).await.unwrap();
    let fetch = h
        .app
        .read_fetch(&scope, job.fetch_id.as_ref().unwrap())
        .await
        .unwrap();
    let dataset = h
        .app
        .create_dataset(&scope, &fetch.id, support::projection(&fetch))
        .await
        .unwrap();
    let view = h
        .app
        .open_view(
            &scope,
            OpenViewRequest {
                dataset_id: dataset.id.clone(),
                kind: ViewKind::DataTable,
            },
        )
        .await
        .unwrap();
    let mut request = send(&c, "two");
    request.selected = vec![
        SelectedReference::Dataset {
            id: dataset.id.clone(),
        },
        SelectedReference::View {
            id: view.id.clone(),
        },
    ];
    let second = host.send(request.clone()).await.unwrap();
    terminal(&host, &c, &second).await;
    let workspace = host.workspace(&c.id).await.unwrap();
    host.mutate_workspace(
        &c.id,
        workspace.revision,
        WorkspaceMutation::Close { view_id: view.id },
    )
    .await
    .unwrap();
    assert_eq!(host.context(&c.id, &second.id).await.unwrap(), second.input);
    assert_eq!(second.input.references.len(), 2);
    assert_eq!(second.input.message_ids.len(), 3);
    let captured = factory.requests.lock().unwrap().clone();
    assert_eq!(captured[1].prompt, second.input.serialized);
    assert!(captured[1].context.is_empty());
    assert!(!captured[1].instructions.contains(&dataset.id));
    assert_eq!(host.send(request).await.unwrap().input, second.input);
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn unsuccessful_provider_receipt_replays_exactly_and_is_not_a_journal_failure() {
    let h = support::Harness::new(
        &[("one", "source_error")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let (behavior, _, _) = Behavior::new("tool_duplicate");
    let host = ConversationHost::start(
        h.app.clone(),
        Arc::new(behavior.clone()),
        ConversationOptions::default(),
    )
    .await
    .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    let run = host.send(send(&c, "one")).await.unwrap();
    assert_eq!(terminal(&host, &c, &run).await.status, RunStatus::Completed);
    let records = host
        .tool_records(&c.id, &run.id, page())
        .await
        .unwrap()
        .items;
    let results = behavior.results.lock().unwrap().clone();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], results[1]);
    assert!(!results[0].success);
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].outcome,
        Some(ToolOutcome::Returned {
            result: results[0].clone()
        })
    );
    let job: JobStatus = serde_json::from_str(&results[0].content).unwrap();
    assert_eq!(job.state, JobState::Failed);
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn closed_old_event_sender_cannot_write_after_new_owner_activates() {
    let h = support::Harness::new(&[], HostBounds::default(), Limits::default()).await;
    let (behavior, _, _) = Behavior::new("retain_events");
    let host = ConversationHost::start(
        h.app.clone(),
        Arc::new(behavior.clone()),
        ConversationOptions::default(),
    )
    .await
    .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    let run = host.send(send(&c, "one")).await.unwrap();
    terminal(&host, &c, &run).await;
    host.shutdown().await.unwrap();
    let replacement = ConversationHost::recover(h.app.clone()).await.unwrap();
    let sender = behavior.late_events.lock().unwrap().clone().unwrap();
    assert!(
        sender
            .send(RuntimeEvent::TextDelta {
                text: "late callback".into()
            })
            .await
            .is_err()
    );
    assert!(
        replacement
            .activity(&c.id, &run.id, page())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        replacement.status(&c.id, &run.id).await.unwrap().status,
        RunStatus::Completed
    );
    replacement.shutdown().await.unwrap();
}
#[tokio::test]
async fn dropped_shutdown_waiter_and_repeated_callers_observe_same_cleanup_failure() {
    let h = support::Harness::new(&[], HostBounds::default(), Limits::default()).await;
    let (behavior, mut closing, release) = Behavior::new("close_gate_error");
    let host = ConversationHost::start(
        h.app.clone(),
        Arc::new(behavior),
        ConversationOptions::default(),
    )
    .await
    .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    let run = host.send(send(&c, "one")).await.unwrap();
    while !*closing.borrow_and_update() {
        closing.changed().await.unwrap();
    }
    let waiter = tokio::spawn({
        let host = host.clone();
        async move { host.shutdown().await }
    });
    tokio::task::yield_now().await;
    waiter.abort();
    assert!(LocalExecutionLease::acquire(&h.application).is_err());
    release.send_replace(true);
    let (a, b) = tokio::join!(host.shutdown(), host.shutdown());
    let error = a.unwrap_err();
    assert_eq!(error.message, "runtime cleanup failed");
    assert_eq!(b.unwrap_err(), error);
    assert_eq!(
        host.status(&c.id, &run.id).await.unwrap().status,
        RunStatus::Failed
    );
    assert!(LocalExecutionLease::acquire(&h.application).is_ok());
}
#[tokio::test]
async fn cancellation_during_factory_creation_is_terminal_without_constructing_runtime() {
    let h = support::Harness::new(&[], HostBounds::default(), Limits::default()).await;
    let (behavior, _, _) = Behavior::new("factory_hold");
    let host = ConversationHost::start(
        h.app.clone(),
        Arc::new(behavior.clone()),
        ConversationOptions::default(),
    )
    .await
    .unwrap();
    let c = host.create("create", "Research").await.unwrap();
    let run = host.send(send(&c, "one")).await.unwrap();
    host.cancel(&c.id, &run.id).await.unwrap();
    assert_eq!(
        terminal(&host, &c, &run).await.status,
        RunStatus::Interrupted
    );
    assert_eq!(behavior.closes.load(Ordering::SeqCst), 0);
    host.shutdown().await.unwrap();
}
