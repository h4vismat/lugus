use lugus_agent::{
    AgentRuntime, Error, RunLimits, RunOutcome, RunRequest, RunSubject, RuntimeEvent, ToolCall,
    ToolExecutor, ToolResult, ToolSpec,
    claude::{ClaudeConfig, ClaudeRuntime},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, watch},
};

struct Executor {
    calls: AtomicUsize,
    large: bool,
    hang: bool,
}
#[async_trait::async_trait]
impl ToolExecutor for Executor {
    async fn execute(&self, call: ToolCall) -> ToolResult {
        assert_eq!(call.run_id, "fixture-run");
        assert_eq!(call.name, "lookup");
        assert_eq!(call.arguments, serde_json::json!({"query":"synthetic"}));
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.hang {
            std::future::pending::<()>().await;
        }
        ToolResult {
            success: true,
            content: if self.large {
                "x".repeat(200)
            } else {
                "fixture value".into()
            },
        }
    }
}
fn executor() -> Executor {
    Executor {
        calls: AtomicUsize::new(0),
        large: false,
        hang: false,
    }
}
fn config(dir: &tempfile::TempDir) -> ClaudeConfig {
    ClaudeConfig {
        executable: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude_cli.py"),
        workspace: dir.path().into(),
        model: None,
    }
}
fn request(scenario: &str) -> RunRequest {
    RunRequest {
        run_id: "fixture-run".into(),
        subject: RunSubject::Conversation {
            id: "synthetic".into(),
        },
        instructions: "Fixture instructions".into(),
        context: "fixture context".into(),
        prompt: scenario.into(),
        allow_web_search: false,
        tools: vec![ToolSpec {
            name: "lookup".into(),
            description: "Synthetic fixture".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }],
        limits: RunLimits {
            timeout: Duration::from_secs(5),
            max_tool_calls: 1,
            max_tool_result_bytes: 100,
        },
    }
}
async fn invoke(
    scenario: &str,
    exec: &Executor,
) -> (
    lugus_agent::Result<lugus_agent::RunReport>,
    Vec<RuntimeEvent>,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = ClaudeRuntime::connect(config(&dir)).await.unwrap();
    let (tx, mut rx) = mpsc::channel(32);
    let (_cancel, cancel) = watch::channel(false);
    let result = runtime.run(request(scenario), exec, tx, cancel).await;
    runtime.close().await.unwrap();
    assert_cleaned(&dir).await;
    let mut events = vec![];
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    (result, events)
}
async fn assert_cleaned(dir: &tempfile::TempDir) {
    if let Ok(url) = fs::read_to_string(dir.path().join("endpoint")) {
        let authorization = fs::read_to_string(dir.path().join("mcp_authorization")).unwrap();
        assert!(
            endpoint_is_inactive(&url, &authorization).await,
            "MCP endpoint leaked"
        );
    }
    #[cfg(unix)]
    if let Ok(pid) = fs::read_to_string(dir.path().join("pid")) {
        let status = std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success(), "CLI process leaked");
    }
}
// A completed TCP handshake can be followed by reset during teardown. Parallel
// runs can also reuse the ephemeral port. Probe this run's capability, rather
// than interpreting any handshake as a live instance of its MCP service.
async fn endpoint_is_inactive(url: &str, authorization: &str) -> bool {
    let addr = url
        .strip_prefix("http://")
        .unwrap()
        .strip_suffix("/mcp")
        .unwrap();
    let probe = async {
        let mut stream = match tokio::net::TcpStream::connect(addr).await {
            Ok(stream) => stream,
            Err(error) => return connection_closed(&error),
        };
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nAuthorization: {authorization}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        if let Err(error) = stream.write_all(request.as_bytes()).await {
            return connection_closed(&error);
        }
        let mut response = Vec::new();
        let read = stream.take(1024).read_to_end(&mut response).await;
        if !response.is_empty() {
            // A new run at this port rejects the old bearer. Any other HTTP
            // response, including a successful ping, still fails the check.
            return response.starts_with(b"HTTP/1.1 401 ");
        }
        match read {
            Ok(_) => true,
            Err(error) => connection_closed(&error),
        }
    };
    tokio::time::timeout(Duration::from_secs(1), probe)
        .await
        .unwrap_or(false)
}
fn connection_closed(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::NotConnected
    )
}

#[tokio::test]
async fn cleanup_probe_accepts_a_connection_reset_after_the_tcp_handshake() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let reset = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        drop(listener);
        drop(socket);
    });
    assert!(endpoint_is_inactive(&url, "Bearer stopped-run").await);
    reset.await.unwrap();
}

#[tokio::test]
async fn cleanup_probe_rejects_a_server_that_still_answers_the_stopped_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = ClaudeRuntime::connect(config(&dir)).await.unwrap();
    let (events, _receive) = mpsc::channel(32);
    let (cancel, cancellation) = watch::channel(false);
    let running = tokio::spawn(async move {
        runtime
            .run(request("hang"), &executor(), events, cancellation)
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !dir.path().join("mcp_authorization").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let url = fs::read_to_string(dir.path().join("endpoint")).unwrap();
    let authorization = fs::read_to_string(dir.path().join("mcp_authorization")).unwrap();
    assert!(!endpoint_is_inactive(&url, &authorization).await);
    cancel.send(true).unwrap();
    running.await.unwrap().unwrap();
    assert_cleaned(&dir).await;
}

#[tokio::test]
async fn streams_once_and_reports_completion_and_usage() {
    let (report, events) = invoke("complete", &executor()).await;
    assert_eq!(report.unwrap().final_text, "Hello world");
    assert!(matches!(&events[0], RuntimeEvent::Started { run_id } if run_id == "fixture-run"));
    assert_eq!(
        events
            .iter()
            .filter_map(|e| match e {
                RuntimeEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>(),
        "Hello world"
    );
    assert!(events.contains(&RuntimeEvent::Usage {
        input_tokens: 7,
        output_tokens: 3
    }));
}
#[tokio::test]
async fn dispatches_mcp_with_host_lifecycle_and_enforces_call_budget() {
    let exec = executor();
    let (report, events) = invoke("tool_limit", &exec).await;
    assert!(report.is_ok(), "{report:?}");
    assert_eq!(exec.calls.load(Ordering::SeqCst), 1);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, RuntimeEvent::ToolStarted { name, .. } if name == "lookup"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, RuntimeEvent::ToolFinished { success: true, .. }))
    );
}
#[tokio::test]
async fn bounds_tool_output_and_rejects_untrusted_http_requests() {
    let exec = Executor {
        large: true,
        ..executor()
    };
    assert!(invoke("tool_large", &exec).await.0.is_ok());
    let exec = executor();
    assert!(invoke("security", &exec).await.0.is_ok());
    assert_eq!(exec.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn malformed_eof_oversized_and_authentication_fail_closed() {
    assert!(matches!(
        invoke("malformed", &executor()).await.0,
        Err(Error::MalformedJson(_))
    ));
    assert!(matches!(
        invoke("oversized_frame", &executor()).await.0,
        Err(Error::FrameTooLarge { .. })
    ));
    assert!(matches!(
        invoke("eof", &executor()).await.0,
        Err(Error::UnexpectedEof)
    ));
    assert!(matches!(
        invoke("auth", &executor()).await.0,
        Err(Error::AuthenticationRequired)
    ));
}
#[tokio::test]
async fn cancellation_and_timeout_interrupt_the_cli_and_tool_executor() {
    for scenario in ["hang", "tool_hang"] {
        for cancelled in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut runtime = ClaudeRuntime::connect(config(&dir)).await.unwrap();
            let (tx, _rx) = mpsc::channel(32);
            let (cancel_tx, cancel_rx) = watch::channel(false);
            let mut req = request(scenario);
            req.limits.timeout = Duration::from_millis(300);
            let trigger = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(150)).await;
                if cancelled {
                    let _ = cancel_tx.send(true);
                }
            });
            let result = runtime
                .run(
                    req,
                    &Executor {
                        hang: true,
                        ..executor()
                    },
                    tx,
                    cancel_rx,
                )
                .await;
            if cancelled {
                assert_eq!(result.unwrap().outcome, RunOutcome::Cancelled);
            } else {
                assert!(matches!(result, Err(Error::Timeout)));
            }
            trigger.await.unwrap();
            assert_cleaned(&dir).await;
        }
    }
}
#[tokio::test]
async fn dropping_the_run_future_releases_process_and_transport() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = ClaudeRuntime::connect(config(&dir)).await.unwrap();
    let (tx, _rx) = mpsc::channel(32);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let exec = executor();
    {
        let future = runtime.run(request("hang"), &exec, tx, cancel_rx);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), future)
                .await
                .is_err()
        );
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_cleaned(&dir).await;
}

#[tokio::test]
async fn zero_tool_budget_blocks_dispatch_and_web_search_is_explicit() {
    for web in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut runtime = ClaudeRuntime::connect(config(&dir)).await.unwrap();
        let (tx, _rx) = mpsc::channel(32);
        let (_cancel, cancel) = watch::channel(false);
        let mut req = request("tool_zero");
        req.allow_web_search = web;
        req.limits.max_tool_calls = 0;
        let exec = executor();
        assert!(runtime.run(req, &exec, tx, cancel).await.is_ok());
        assert_eq!(exec.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn event_consumer_failure_cleans_up_and_closed_runtime_rejects_runs() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = ClaudeRuntime::connect(config(&dir)).await.unwrap();
    let (tx, rx) = mpsc::channel(1);
    drop(rx);
    let (_cancel, cancel) = watch::channel(false);
    assert!(matches!(
        runtime
            .run(request("complete"), &executor(), tx, cancel)
            .await,
        Err(Error::EventConsumerDisconnected)
    ));
    assert_cleaned(&dir).await;
    runtime.close().await.unwrap();
    let (tx, _rx) = mpsc::channel(1);
    let (_cancel, cancel) = watch::channel(false);
    assert!(matches!(
        runtime
            .run(request("complete"), &executor(), tx, cancel)
            .await,
        Err(Error::Process(_))
    ));
}

#[tokio::test]
async fn validates_configuration_and_rejects_unsupported_cli() {
    let dir = tempfile::tempdir().unwrap();
    let mut invalid = config(&dir);
    invalid.workspace = dir.path().join("missing");
    assert!(matches!(
        ClaudeRuntime::connect(invalid).await,
        Err(Error::Configuration(_))
    ));
    let mut invalid = config(&dir);
    invalid.model = Some("\n".into());
    assert!(matches!(
        ClaudeRuntime::connect(invalid).await,
        Err(Error::Configuration(_))
    ));
    let mut invalid = config(&dir);
    invalid.executable = "/usr/bin/true".into();
    assert!(matches!(
        ClaudeRuntime::connect(invalid).await,
        Err(Error::Configuration(_))
    ));
}

#[tokio::test]
async fn informational_tool_progress_does_not_terminate_a_valid_run() {
    let (report, _) = invoke("progress", &executor()).await;
    assert_eq!(report.unwrap().outcome, RunOutcome::Completed);
}
