use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use lugus_agent::codex::{AccountStatus, CodexConfig, CodexRuntime};
use lugus_agent::{
    AgentRuntime, Error, RunLimits, RunOutcome, RunRequest, RuntimeEvent, ToolCall, ToolExecutor,
    ToolResult, ToolSpec,
};
use tempfile::TempDir;
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

struct FakeExecutable {
    _directory: TempDir,
    path: PathBuf,
}

impl FakeExecutable {
    fn for_scenario(scenario: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(scenario);
        fs::copy(fixture(), &path).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        Self {
            _directory: directory,
            path,
        }
    }

    fn config(&self, model: Option<&str>, provider: Option<&str>) -> CodexConfig {
        CodexConfig {
            executable: self.path.clone(),
            workspace: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            model: model.map(str::to_owned),
            model_provider: provider.map(str::to_owned),
        }
    }
}

#[derive(Default)]
struct RecallExecutor {
    calls: Mutex<Vec<ToolCall>>,
}

struct PendingExecutor;

#[async_trait::async_trait]
impl ToolExecutor for PendingExecutor {
    async fn execute(&self, _call: ToolCall) -> ToolResult {
        std::future::pending().await
    }
}

struct OversizedExecutor;

#[async_trait::async_trait]
impl ToolExecutor for OversizedExecutor {
    async fn execute(&self, _call: ToolCall) -> ToolResult {
        ToolResult {
            success: true,
            content: "this result is deliberately too large".into(),
        }
    }
}

#[async_trait::async_trait]
impl ToolExecutor for RecallExecutor {
    async fn execute(&self, call: ToolCall) -> ToolResult {
        let valid = call.name == "lugus_recall"
            && call.arguments == serde_json::json!({"thesis_id": "thesis-A"});
        self.calls.lock().unwrap().push(call);
        ToolResult {
            success: valid,
            content: if valid {
                "Stored finding from thesis A".into()
            } else {
                "invalid arguments".into()
            },
        }
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex_server.py")
}

fn request(run_id: &str) -> RunRequest {
    RunRequest {
        run_id: run_id.into(),
        thesis_id: "thesis-A".into(),
        instructions: "You are the Lugus review agent.".into(),
        context: "Use only the supplied thesis context.".into(),
        prompt: "Review thesis A".into(),
        tools: vec![ToolSpec {
            name: "lugus_recall".into(),
            description: "Recall stored findings".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"thesis_id": {"type": "string"}},
                "required": ["thesis_id"],
                "additionalProperties": false
            }),
        }],
        limits: RunLimits {
            timeout: Duration::from_secs(2),
            max_tool_calls: 1,
            max_tool_result_bytes: 1024,
        },
    }
}

async fn run(
    runtime: &mut CodexRuntime,
    request: RunRequest,
    executor: &dyn ToolExecutor,
) -> (
    lugus_agent::Result<lugus_agent::RunReport>,
    Vec<RuntimeEvent>,
) {
    let (events, mut received) = mpsc::channel(16);
    let (_cancel_tx, cancel) = watch::channel(false);
    let report = runtime.run(request, executor, events, cancel).await;
    let mut captured = Vec::new();
    while let Ok(event) = received.try_recv() {
        captured.push(event);
    }
    (report, captured)
}

async fn assert_runtime_was_reaped(runtime: &mut CodexRuntime) {
    let (events, _received) = mpsc::channel(1);
    let (_cancel_tx, cancel) = watch::channel(false);
    let result = timeout(
        Duration::from_millis(100),
        runtime.run(
            request("run-after-cancellation"),
            &RecallExecutor::default(),
            events,
            cancel,
        ),
    )
    .await
    .expect("a reaped runtime rejects a new run immediately");
    assert!(matches!(result, Err(Error::Process(_))));
}

#[tokio::test]
async fn initializes_dispatches_a_host_tool_and_completes_with_final_text() {
    let executable = FakeExecutable::for_scenario("normal");
    let mut runtime =
        CodexRuntime::connect(executable.config(Some("gpt-test"), Some("test-provider")))
            .await
            .unwrap();
    assert_eq!(
        runtime.account_status().await.unwrap(),
        AccountStatus::Ready
    );

    let executor = RecallExecutor::default();
    let (report, events) = run(&mut runtime, request("host-run-7"), &executor).await;
    let report = report.unwrap();

    assert_eq!(report.run_id, "host-run-7");
    assert_eq!(report.outcome, RunOutcome::Completed);
    assert_eq!(
        report.final_text,
        "Assessment uses stored finding from thesis A."
    );
    assert_eq!(
        executor.calls.lock().unwrap().as_slice(),
        &[ToolCall {
            run_id: "host-run-7".into(),
            call_id: "call-1".into(),
            name: "lugus_recall".into(),
            arguments: serde_json::json!({"thesis_id": "thesis-A"}),
        }]
    );
    assert_eq!(
        events,
        vec![
            RuntimeEvent::Started {
                run_id: "host-run-7".into()
            },
            RuntimeEvent::ToolStarted {
                call_id: "call-1".into(),
                name: "lugus_recall".into()
            },
            RuntimeEvent::ToolFinished {
                call_id: "call-1".into(),
                success: true
            },
            RuntimeEvent::TextDelta {
                text: "Assessment uses ".into()
            },
            RuntimeEvent::TextDelta {
                text: "stored finding from thesis A.".into()
            },
        ]
    );
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn reports_login_required_without_starting_login() {
    let executable = FakeExecutable::for_scenario("login_required");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();

    assert_eq!(
        runtime.account_status().await.unwrap(),
        AccountStatus::LoginRequired
    );
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn accepts_a_provider_that_does_not_require_openai_auth() {
    let executable = FakeExecutable::for_scenario("provider_auth");
    let mut runtime = CodexRuntime::connect(executable.config(None, Some("compatible")))
        .await
        .unwrap();

    assert_eq!(
        runtime.account_status().await.unwrap(),
        AccountStatus::Ready
    );
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn returns_rpc_errors_from_thread_start() {
    let executable = FakeExecutable::for_scenario("rpc_error");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();

    let (result, _) = run(&mut runtime, request("run-rpc"), &RecallExecutor::default()).await;
    assert!(matches!(result, Err(Error::Protocol(message)) if message.contains("thread rejected")));
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn rejects_tool_calls_for_a_different_thread() {
    let executable = FakeExecutable::for_scenario("wrong_thread");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();
    let executor = RecallExecutor::default();

    let (result, _) = run(&mut runtime, request("run-wrong-thread"), &executor).await;
    assert!(matches!(result, Err(Error::Protocol(message)) if message.contains("active thread")));
    assert!(executor.calls.lock().unwrap().is_empty());
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn unknown_tools_receive_a_failure_without_calling_the_host() {
    let executable = FakeExecutable::for_scenario("unknown_tool");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();
    let executor = RecallExecutor::default();

    let (report, events) = run(&mut runtime, request("run-unknown"), &executor).await;
    assert_eq!(report.unwrap().outcome, RunOutcome::Completed);
    assert!(executor.calls.lock().unwrap().is_empty());
    assert!(events.contains(&RuntimeEvent::ToolFinished {
        call_id: "call-1".into(),
        success: false,
    }));
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn namespaced_calls_cannot_invoke_a_registered_flat_tool() {
    let executable = FakeExecutable::for_scenario("namespaced_tool");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();
    let executor = RecallExecutor::default();

    let (report, events) = run(&mut runtime, request("run-namespaced"), &executor).await;

    assert_eq!(report.unwrap().outcome, RunOutcome::Completed);
    assert!(executor.calls.lock().unwrap().is_empty());
    assert!(!events.iter().any(|event| matches!(
        event,
        RuntimeEvent::ToolStarted { .. } | RuntimeEvent::ToolFinished { .. }
    )));
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn final_text_fallback_uses_only_the_last_assistant_item() {
    let executable = FakeExecutable::for_scenario("multiple_messages");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();

    let (report, _) = run(
        &mut runtime,
        request("run-multiple-messages"),
        &RecallExecutor::default(),
    )
    .await;

    assert_eq!(report.unwrap().final_text, "Final response.");
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn rejects_an_unsupported_codex_version() {
    let executable = FakeExecutable::for_scenario("bad_version");
    let error = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap_err();

    assert!(matches!(error, Error::Configuration(message) if message.contains("0.153.4")));
}

#[tokio::test]
async fn bounds_codex_version_output() {
    let executable = FakeExecutable::for_scenario("oversized_version");
    let error = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap_err();

    assert!(matches!(error, Error::Process(message) if message.contains("version output")));
}

#[tokio::test]
async fn cancellation_before_startup_returns_a_cancelled_report() {
    let executable = FakeExecutable::for_scenario("interruptible");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();
    let (events, _received) = mpsc::channel(1);
    let (cancel_tx, cancel) = watch::channel(true);

    let report = runtime
        .run(
            request("run-cancel-before-start"),
            &RecallExecutor::default(),
            events,
            cancel,
        )
        .await
        .unwrap();

    assert_eq!(report.outcome, RunOutcome::Cancelled);
    drop(cancel_tx);
    assert_runtime_was_reaped(&mut runtime).await;
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn cancellation_interrupts_an_active_turn_and_reaps_the_process() {
    let executable = FakeExecutable::for_scenario("interruptible");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();
    let (events, mut received) = mpsc::channel(4);
    let (cancel_tx, cancel) = watch::channel(false);
    let executor = RecallExecutor::default();
    let report = {
        let run = runtime.run(request("run-cancel-active"), &executor, events, cancel);
        tokio::pin!(run);

        tokio::select! {
            event = received.recv() => assert!(matches!(event, Some(RuntimeEvent::Started { .. }))),
            result = &mut run => panic!("run ended before Started: {result:?}"),
        }
        cancel_tx.send(true).unwrap();
        timeout(Duration::from_millis(500), &mut run)
            .await
            .expect("cancellation must not wait for the run deadline")
            .unwrap()
    };

    assert_eq!(report.outcome, RunOutcome::Cancelled);
    assert_runtime_was_reaped(&mut runtime).await;
    runtime.close().await.unwrap();
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn refreshes_inherited_mcp_ids_before_each_thread_start() {
    let executable = FakeExecutable::for_scenario("refresh_config");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();
    let executor = RecallExecutor::default();

    assert_eq!(
        run(&mut runtime, request("run-refresh-one"), &executor)
            .await
            .0
            .unwrap()
            .outcome,
        RunOutcome::Completed,
    );
    assert_eq!(
        run(&mut runtime, request("run-refresh-two"), &executor)
            .await
            .0
            .unwrap()
            .outcome,
        RunOutcome::Completed,
    );
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn cancellation_remains_responsive_while_a_tool_executor_is_pending() {
    let executable = FakeExecutable::for_scenario("pending_tool");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();
    let (events, mut received) = mpsc::channel(4);
    let (cancel_tx, cancel) = watch::channel(false);
    let report = {
        let run = runtime.run(request("run-cancel-tool"), &PendingExecutor, events, cancel);
        tokio::pin!(run);

        tokio::select! {
            event = received.recv() => assert!(matches!(event, Some(RuntimeEvent::Started { .. }))),
            result = &mut run => panic!("run ended before Started: {result:?}"),
        }
        tokio::select! {
            event = received.recv() => assert!(matches!(event, Some(RuntimeEvent::ToolStarted { .. }))),
            result = &mut run => panic!("run ended before ToolStarted: {result:?}"),
        }
        cancel_tx.send(true).unwrap();
        timeout(Duration::from_millis(500), &mut run)
            .await
            .expect("cancellation must interrupt a pending host tool")
            .unwrap()
    };

    assert_eq!(report.outcome, RunOutcome::Cancelled);
    assert_runtime_was_reaped(&mut runtime).await;
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn cancellation_during_startup_reaps_before_any_turn_is_started() {
    let executable = FakeExecutable::for_scenario("startup_stall");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();
    let (events, _received) = mpsc::channel(1);
    let (cancel_tx, cancel) = watch::channel(false);
    let executor = RecallExecutor::default();
    let report = {
        let run = runtime.run(request("run-cancel-startup"), &executor, events, cancel);
        tokio::pin!(run);

        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(25)) => cancel_tx.send(true).unwrap(),
            result = &mut run => panic!("startup ended before cancellation: {result:?}"),
        }
        timeout(Duration::from_millis(500), &mut run)
            .await
            .expect("startup cancellation must not wait for a transport deadline")
            .unwrap()
    };

    assert_eq!(report.outcome, RunOutcome::Cancelled);
    assert_runtime_was_reaped(&mut runtime).await;
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn startup_accepts_a_response_within_the_existing_thirty_second_budget() {
    let executable = FakeExecutable::for_scenario("slow_initialize");
    let mut runtime = timeout(
        Duration::from_secs(10),
        CodexRuntime::connect(executable.config(None, None)),
    )
    .await
    .expect("the delayed response should complete within the test deadline")
    .expect("a six-second response remains within the supported RPC budget");
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn startup_rpcs_timeout_across_unrelated_notifications() {
    let executable = FakeExecutable::for_scenario("startup_notifications");
    let error = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap_err();

    assert!(matches!(error, Error::Timeout));
}

#[tokio::test]
async fn refuses_to_start_when_inherited_mcp_configuration_cannot_be_parsed() {
    let executable = FakeExecutable::for_scenario("malformed_config");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();
    let (result, _) = run(
        &mut runtime,
        request("run-malformed-config"),
        &RecallExecutor::default(),
    )
    .await;

    assert!(
        matches!(result, Err(Error::Configuration(message)) if message.contains("mcp_servers"))
    );
}

#[tokio::test]
async fn approval_requests_are_cancelled_and_report_needs_attention() {
    let executable = FakeExecutable::for_scenario("approval");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();

    let (result, _) = run(
        &mut runtime,
        request("run-approval"),
        &RecallExecutor::default(),
    )
    .await;
    assert!(matches!(result, Err(Error::NeedsAttention(message)) if message.contains("approval")));
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn human_input_requests_fail_with_needs_attention() {
    let executable = FakeExecutable::for_scenario("human_input");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();

    let (result, _) = run(
        &mut runtime,
        request("run-human-input"),
        &RecallExecutor::default(),
    )
    .await;
    assert!(
        matches!(result, Err(Error::NeedsAttention(message)) if message.contains("human input"))
    );
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn tool_call_exhaustion_returns_a_typed_error_without_starting_the_excess_call() {
    let executable = FakeExecutable::for_scenario("tool_limit");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();

    let (result, events) = run(
        &mut runtime,
        request("run-tool-limit"),
        &RecallExecutor::default(),
    )
    .await;
    assert!(matches!(result, Err(Error::Tool(message)) if message.contains("limit")));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RuntimeEvent::ToolStarted { .. }))
            .count(),
        1,
    );
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn closed_event_consumers_fail_explicitly() {
    let executable = FakeExecutable::for_scenario("interruptible");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();
    let (events, received) = mpsc::channel(1);
    drop(received);
    let (_cancel_tx, cancel) = watch::channel(false);

    let result = runtime
        .run(
            request("run-closed-events"),
            &RecallExecutor::default(),
            events,
            cancel,
        )
        .await;
    assert!(matches!(result, Err(Error::EventConsumerDisconnected)));
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn slow_event_consumers_are_bounded() {
    let executable = FakeExecutable::for_scenario("multiple_messages");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();
    let (events, _received) = mpsc::channel(1);
    let (_cancel_tx, cancel) = watch::channel(false);

    let result = runtime
        .run(
            request("run-slow-events"),
            &RecallExecutor::default(),
            events,
            cancel,
        )
        .await;
    assert!(matches!(result, Err(Error::EventConsumerSlow)));
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn exhausted_run_time_returns_timeout_and_reaps_the_process() {
    let executable = FakeExecutable::for_scenario("interruptible");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();
    let mut timed_request = request("run-timeout");
    timed_request.limits.timeout = Duration::from_millis(30);

    let (result, _) = run(&mut runtime, timed_request, &RecallExecutor::default()).await;
    assert!(matches!(result, Err(Error::Timeout)));
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn oversized_tool_results_are_replaced_with_a_bounded_failure() {
    let executable = FakeExecutable::for_scenario("oversized_result");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();
    let mut bounded_request = request("run-oversized-result");
    bounded_request.limits.max_tool_result_bytes = 5;

    let (report, _) = run(&mut runtime, bounded_request, &OversizedExecutor).await;
    assert_eq!(report.unwrap().outcome, RunOutcome::Completed);
    runtime.close().await.unwrap();
}

#[tokio::test]
async fn process_death_during_a_turn_is_reported_explicitly() {
    let executable = FakeExecutable::for_scenario("process_death");
    let mut runtime = CodexRuntime::connect(executable.config(None, None))
        .await
        .unwrap();

    let (result, _) = run(
        &mut runtime,
        request("run-process-death"),
        &RecallExecutor::default(),
    )
    .await;
    assert!(matches!(result, Err(Error::UnexpectedEof)));
    runtime.close().await.unwrap();
}
