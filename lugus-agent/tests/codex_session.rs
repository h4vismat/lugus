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
