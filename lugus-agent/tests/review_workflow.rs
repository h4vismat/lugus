use lugus_agent::{reviews::*, *};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, watch};
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        "2026-09-10T12:00:00Z".parse().unwrap()
    }
}
#[derive(Clone, Copy)]
enum Behavior {
    Submit,
    SubmitChanged,
    NoSubmit,
    Cancel,
    Fail,
    SubmitFail,
    SubmitCancel,
    Edit,
    Scope,
    Wait,
    OversizedError,
}
struct Runtime {
    behavior: Behavior,
    store: Arc<SqliteReviewStore>,
    closed: Arc<AtomicBool>,
    saw_previous: bool,
}
#[async_trait::async_trait]
impl AgentRuntime for Runtime {
    async fn run(
        &mut self,
        r: RunRequest,
        tools: &dyn ToolExecutor,
        _events: mpsc::Sender<RuntimeEvent>,
        _cancel: watch::Receiver<bool>,
    ) -> Result<RunReport> {
        assert!(!r.allow_web_search);
        assert!(r.context.is_empty());
        assert!(!r.instructions.contains("USER TEXT"));
        let input: Value = serde_json::from_str(&r.prompt).unwrap();
        assert_eq!(
            input["thesis"]["text"],
            "USER TEXT: assets may imply resilience"
        );
        self.saw_previous = !input["previous_assessment"].is_null();
        let eid = input["evidence"][0]["id"].as_str().unwrap();
        let call = |name: &str, args: Value| ToolCall {
            run_id: r.run_id.clone(),
            call_id: "wire-call".into(),
            name: name.into(),
            arguments: args,
        };
        if matches!(self.behavior, Behavior::OversizedError) {
            let args = json!({"id":eid,"x".repeat(10000):true});
            let result = tools.execute(call("lugus_read_evidence", args)).await;
            assert!(!result.success);
            assert!(
                result.content.len() <= 1024,
                "oversized error: {} bytes",
                result.content.len()
            );
            return Ok(RunReport {
                run_id: r.run_id,
                outcome: RunOutcome::Completed,
                final_text: String::new(),
            });
        }
        if matches!(self.behavior, Behavior::Wait) {
            std::future::pending::<()>().await;
        }
        if matches!(self.behavior, Behavior::Cancel) {
            return Ok(RunReport {
                run_id: r.run_id,
                outcome: RunOutcome::Cancelled,
                final_text: String::new(),
            });
        }
        if matches!(self.behavior, Behavior::Fail) {
            return Err(Error::AuthenticationRequired);
        }
        if matches!(self.behavior, Behavior::NoSubmit) {
            return Ok(RunReport {
                run_id: r.run_id,
                outcome: RunOutcome::Completed,
                final_text: "Looks complete".into(),
            });
        }
        if matches!(self.behavior, Behavior::Scope) {
            assert!(
                !tools
                    .execute(call("lugus_read_evidence", json!({"id":"other-review"})))
                    .await
                    .success
            );
            let mut wrong = call("lugus_read_evidence", json!({"id":eid}));
            wrong.run_id = "other-run".into();
            assert!(!tools.execute(wrong).await.success);
            assert!(
                !tools
                    .execute(call("edit_thesis", json!({"text":"hacked"})))
                    .await
                    .success
            );
            assert!(
                !tools
                    .execute(call("lugus_read_evidence", json!({"id":eid,"extra":true})))
                    .await
                    .success
            );
        }
        let read = tools
            .execute(call("lugus_read_evidence", json!({"id":eid})))
            .await;
        assert!(read.success, "{}", read.content);
        let ev: Value = serde_json::from_str(&read.content).unwrap();
        assert_eq!(
            ev["content"]["value"],
            if matches!(self.behavior, Behavior::SubmitChanged) {
                "80"
            } else {
                "100"
            }
        );
        if matches!(self.behavior, Behavior::Edit) {
            self.store
                .save_thesis("t", 1, "Updated user thesis", FixedClock.now())
                .unwrap();
        }
        let draft = json!({"interpretation":"Assets may imply resilience","conclusion":"Insufficient evidence","supporting":[{"text":if matches!(self.behavior,Behavior::SubmitChanged){"Assets are 80"}else{"Assets are 100"},"evidence_ids":[eid]}],"opposing":[],"uncertainty":["Missing liabilities"],"open_questions":["What are liabilities?"],"changes":if self.saw_previous {"Reconsidered previous assessment"}else{"First assessment"}});
        let submission = tools
            .execute(call("lugus_submit_assessment", draft.clone()))
            .await;
        if matches!(self.behavior, Behavior::Edit) {
            assert!(!submission.success);
        } else {
            assert!(submission.success, "{}", submission.content);
            assert!(
                tools
                    .execute(call("lugus_submit_assessment", draft))
                    .await
                    .success
            );
        }
        if matches!(self.behavior, Behavior::SubmitFail) {
            return Err(Error::Process("disconnected".into()));
        }
        Ok(RunReport {
            run_id: r.run_id,
            outcome: if matches!(self.behavior, Behavior::SubmitCancel) {
                RunOutcome::Cancelled
            } else {
                RunOutcome::Completed
            },
            final_text: String::new(),
        })
    }
    async fn close(&mut self) -> Result<()> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}
fn setup() -> (tempfile::TempDir, Arc<SqliteReviewStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteReviewStore::open(dir.path().join("agent.db")).unwrap());
    store
        .save_thesis(
            "t",
            0,
            "USER TEXT: assets may imply resilience",
            FixedClock.now(),
        )
        .unwrap();
    store
        .enqueue(
            "r",
            "t",
            1,
            vec![Evidence::new("financial:one", "Assets", json!({"value":"100"})).unwrap()],
            FixedClock.now(),
        )
        .unwrap();
    (dir, store)
}
async fn execute(store: Arc<SqliteReviewStore>, behavior: Behavior, id: &str) -> (Review, bool) {
    let closed = Arc::new(AtomicBool::new(false));
    let mut runtime = Runtime {
        behavior,
        store: store.clone(),
        closed: closed.clone(),
        saw_previous: false,
    };
    let (_cancel, rx) = watch::channel(false);
    let (events, _receiver) = mpsc::channel(32);
    let options = ReviewExecution {
        review_id: id.into(),
        runtime_identity: "deterministic:v1".into(),
        limits: RunLimits {
            timeout: Duration::from_millis(100),
            max_tool_calls: 20,
            max_tool_result_bytes: if matches!(behavior, Behavior::OversizedError) {
                1024
            } else {
                131072
            },
        },
    };
    let result = ReviewCoordinator::new(store.as_ref(), &FixedClock)
        .execute(&mut runtime, options, events, rx)
        .await
        .unwrap();
    assert!(closed.load(Ordering::SeqCst));
    (result, runtime.saw_previous)
}
#[tokio::test]
async fn fresh_runtime_after_reopen_receives_previous_assessment() {
    let (dir, store) = setup();
    assert_eq!(
        execute(store.clone(), Behavior::Submit, "r").await.0.status,
        ReviewStatus::Completed
    );
    drop(store);
    let store = Arc::new(SqliteReviewStore::open(dir.path().join("agent.db")).unwrap());
    let evidence = vec![Evidence::new("financial:two", "Assets", json!({"value":"80"})).unwrap()];
    store
        .enqueue("r2", "t", 1, evidence, FixedClock.now())
        .unwrap();
    let (review, saw_previous) = execute(store.clone(), Behavior::SubmitChanged, "r2").await;
    assert_eq!(
        store.review("r").unwrap().evidence[0].content["value"],
        "100"
    );
    assert_eq!(review.evidence[0].content["value"], "80");
    assert!(saw_previous);
    assert_eq!(review.status, ReviewStatus::Completed);
    assert_eq!(store.assessments("t").unwrap().len(), 2);
}
#[tokio::test]
async fn only_a_committed_assessment_completes_a_review() {
    for (behavior, expected) in [
        (Behavior::NoSubmit, ReviewStatus::Failed),
        (Behavior::Cancel, ReviewStatus::Interrupted),
        (Behavior::Fail, ReviewStatus::Blocked),
        (Behavior::SubmitFail, ReviewStatus::Completed),
        (Behavior::SubmitCancel, ReviewStatus::Completed),
        (Behavior::Edit, ReviewStatus::Blocked),
        (Behavior::Scope, ReviewStatus::Completed),
        (Behavior::Wait, ReviewStatus::Failed),
    ] {
        let (_dir, store) = setup();
        let (review, _) = execute(store.clone(), behavior, "r").await;
        assert_eq!(review.status, expected);
        assert_eq!(
            store.assessments("t").unwrap().len(),
            usize::from(expected == ReviewStatus::Completed)
        );
    }
}
#[tokio::test]
async fn retry_uses_new_attempt_and_preserves_inputs() {
    let (_dir, store) = setup();
    execute(store.clone(), Behavior::Fail, "r").await;
    let (review, _) = execute(store.clone(), Behavior::Submit, "r").await;
    assert_eq!(review.status, ReviewStatus::Completed);
    assert_eq!(review.attempt, 2);
}

#[tokio::test]
async fn tool_errors_obey_the_host_result_budget() {
    let (_dir, store) = setup();
    assert_eq!(
        execute(store, Behavior::OversizedError, "r").await.0.status,
        ReviewStatus::Failed
    );
}

struct AwaitCancellation {
    committed: bool,
    started: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
}
#[async_trait::async_trait]
impl AgentRuntime for AwaitCancellation {
    async fn run(
        &mut self,
        r: RunRequest,
        tools: &dyn ToolExecutor,
        _events: mpsc::Sender<RuntimeEvent>,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<RunReport> {
        if self.committed {
            let draft = json!({"interpretation":"Cannot assess","conclusion":"Insufficient evidence","supporting":[],"opposing":[],"uncertainty":["Missing facts"],"open_questions":["Which facts matter?"],"changes":"First review"});
            let result = tools
                .execute(ToolCall {
                    run_id: r.run_id.clone(),
                    call_id: "submit".into(),
                    name: "lugus_submit_assessment".into(),
                    arguments: draft,
                })
                .await;
            assert!(result.success);
        }
        self.started.store(true, Ordering::SeqCst);
        while !*cancel.borrow() {
            if cancel.changed().await.is_err() {
                break;
            }
        }
        self.started.store(false, Ordering::SeqCst);
        Ok(RunReport {
            run_id: r.run_id,
            outcome: RunOutcome::Cancelled,
            final_text: String::new(),
        })
    }
    async fn close(&mut self) -> Result<()> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}
async fn wait_started(started: &AtomicBool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn host_cancellation_preserves_only_already_committed_assessments() {
    for committed in [false, true] {
        let (_dir, store) = setup();
        let started = Arc::new(AtomicBool::new(false));
        let closed = Arc::new(AtomicBool::new(false));
        let mut runtime = AwaitCancellation {
            committed,
            started: started.clone(),
            closed: closed.clone(),
        };
        let (sender, cancel) = watch::channel(false);
        let (events, _receiver) = mpsc::channel(4);
        let worker_store = store.clone();
        let task = tokio::spawn(async move {
            ReviewCoordinator::new(worker_store.as_ref(), &FixedClock)
                .execute(
                    &mut runtime,
                    ReviewExecution {
                        review_id: "r".into(),
                        runtime_identity: "fake".into(),
                        limits: RunLimits {
                            timeout: Duration::from_secs(30),
                            max_tool_calls: 4,
                            max_tool_result_bytes: 131072,
                        },
                    },
                    events,
                    cancel,
                )
                .await
                .unwrap()
        });
        wait_started(&started).await;
        sender.send(true).unwrap();
        let review = task.await.unwrap();
        assert_eq!(
            review.status,
            if committed {
                ReviewStatus::Completed
            } else {
                ReviewStatus::Interrupted
            }
        );
        assert_eq!(
            store.assessments("t").unwrap().len(),
            usize::from(committed)
        );
        assert!(closed.load(Ordering::SeqCst));
        assert!(
            !started.load(Ordering::SeqCst),
            "runtime must get a chance to acknowledge cancellation"
        );
    }
}
#[tokio::test]
async fn dropped_execution_is_recoverable_with_a_fresh_runtime() {
    let (_dir, store) = setup();
    let started = Arc::new(AtomicBool::new(false));
    let closed = Arc::new(AtomicBool::new(false));
    let mut runtime = AwaitCancellation {
        committed: false,
        started: started.clone(),
        closed,
    };
    let (_sender, cancel) = watch::channel(false);
    let (events, _receiver) = mpsc::channel(4);
    let worker_store = store.clone();
    let task = tokio::spawn(async move {
        ReviewCoordinator::new(worker_store.as_ref(), &FixedClock)
            .execute(
                &mut runtime,
                ReviewExecution {
                    review_id: "r".into(),
                    runtime_identity: "fake".into(),
                    limits: RunLimits {
                        timeout: Duration::from_secs(30),
                        max_tool_calls: 4,
                        max_tool_result_bytes: 131072,
                    },
                },
                events,
                cancel,
            )
            .await
    });
    wait_started(&started).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(store.review("r").unwrap().status, ReviewStatus::Running);
    assert_eq!(store.recover(FixedClock.now()).unwrap(), 1);
    let (review, _) = execute(store.clone(), Behavior::Submit, "r").await;
    assert_eq!(review.status, ReviewStatus::Completed);
    assert_eq!(review.attempt, 2);
}
