use super::mcp::{Invocation, McpServer};
use crate::{
    AgentRuntime, Error, Result, RunOutcome, RunReport, RunRequest, RuntimeEvent, ToolCall,
    ToolExecutor, ToolResult, validate_request,
};
use serde_json::{Value, json};
use std::{collections::HashSet, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{ChildStdout, Command},
    sync::{mpsc, watch},
};

const MAX_FRAME: usize = 8 * 1024 * 1024;
const MAX_INPUT: usize = 8 * 1024 * 1024;
const MAX_TEXT: usize = 8 * 1024 * 1024;
const EVENT_TIMEOUT: Duration = Duration::from_millis(100);
const SUPPORTED_VERSION: &str = "2.1.268 (Claude Code)";

#[derive(Debug, Clone)]
pub struct ClaudeConfig {
    pub executable: PathBuf,
    pub workspace: PathBuf,
    pub model: Option<String>,
}
#[derive(Debug)]
pub struct ClaudeRuntime {
    config: ClaudeConfig,
    closed: bool,
}
impl ClaudeRuntime {
    /// Validate configuration and the CLI protocol version without accessing credentials.
    pub async fn connect(mut config: ClaudeConfig) -> Result<Self> {
        if config.executable.as_os_str().is_empty() {
            return Err(Error::Configuration(
                "Claude executable must not be empty".into(),
            ));
        }
        config.workspace = config
            .workspace
            .canonicalize()
            .map_err(|_| Error::Configuration("Claude workspace does not exist".into()))?;
        if !config.workspace.is_dir() {
            return Err(Error::Configuration(
                "Claude workspace must be a directory".into(),
            ));
        }
        if config.model.as_ref().is_some_and(|s| {
            s.trim().is_empty() || s.len() > 256 || s.chars().any(char::is_control)
        }) {
            return Err(Error::Configuration(
                "Claude model must be bounded control-free text".into(),
            ));
        }
        if config.executable.components().count() > 1 {
            config.executable = config
                .executable
                .canonicalize()
                .map_err(|_| Error::Configuration("Claude executable does not exist".into()))?;
        }
        let mut child = Command::new(&config.executable)
            .arg("--version")
            .current_dir(&config.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(process_error)?;
        let mut stdout = child.stdout.take().ok_or_else(missing_pipe)?.take(257);
        let check = async {
            let mut output = Vec::new();
            stdout
                .read_to_end(&mut output)
                .await
                .map_err(process_error)?;
            if output.len() > 256 {
                return Err(Error::Configuration(
                    "Claude version output is oversized".into(),
                ));
            }
            let status = child.wait().await.map_err(process_error)?;
            if !status.success() || String::from_utf8_lossy(&output).trim() != SUPPORTED_VERSION {
                return Err(Error::Configuration(format!(
                    "Claude Code {SUPPORTED_VERSION} is required by this adapter"
                )));
            }
            Ok(())
        };
        tokio::time::timeout(Duration::from_secs(5), check)
            .await
            .map_err(|_| Error::Timeout)??;
        Ok(Self {
            config,
            closed: false,
        })
    }
}
#[async_trait::async_trait]
impl AgentRuntime for ClaudeRuntime {
    async fn run(
        &mut self,
        request: RunRequest,
        tools: &dyn ToolExecutor,
        events: mpsc::Sender<RuntimeEvent>,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<RunReport> {
        if self.closed {
            return Err(Error::Process("Claude runtime is closed".into()));
        }
        validate_request(&request)?;
        let input = json!({"context": request.context, "prompt": request.prompt}).to_string();
        if input.len() > MAX_INPUT
            || request.instructions.len() > MAX_INPUT
            || serde_json::to_vec(&request.tools)
                .map_err(|_| Error::InvalidRequest("cannot encode tool specs".into()))?
                .len()
                > 1024 * 1024
        {
            return Err(Error::InvalidRequest(
                "Claude request exceeds input limit".into(),
            ));
        }
        if *cancel.borrow() {
            return Ok(cancelled(&request));
        }
        let deadline = tokio::time::Instant::now() + request.limits.timeout;
        let mut server =
            McpServer::start(request.tools.clone(), request.limits.max_tool_calls).await?;
        let mut command = Command::new(&self.config.executable);
        command
            .args(startup_args(
                &request,
                &server.config,
                self.config.model.as_deref(),
            ))
            .current_dir(&self.config.workspace)
            .env_remove("CLAUDE_CODE_SIMPLE")
            .env_remove("CLAUDE_CODE_SAFE_MODE")
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
            .env("ENABLE_TOOL_SEARCH", "false")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(process_error)?;
        let mut stdin = child.stdin.take().ok_or_else(missing_pipe)?;
        let stdout = child.stdout.take().ok_or_else(missing_pipe)?;
        let operation = async {
            emit(
                &events,
                RuntimeEvent::Started {
                    run_id: request.run_id.clone(),
                },
            )
            .await?;
            stdin
                .write_all(input.as_bytes())
                .await
                .map_err(process_error)?;
            stdin.shutdown().await.map_err(process_error)?;
            drop(stdin);
            let report = drive(stdout, &mut server, &request, tools, &events).await?;
            if !child.wait().await.map_err(process_error)?.success() {
                return Err(Error::Process("Claude exited unsuccessfully".into()));
            }
            Ok(report)
        };
        let result = tokio::select! {
            biased;
            () = cancelled_signal(&mut cancel) => Ok(cancelled(&request)),
            () = tokio::time::sleep_until(deadline) => Err(Error::Timeout),
            result = operation => result,
        };
        // kill_on_drop also covers callers dropping run() itself. No tasks hold tools.
        let _ = child.kill().await;
        let _ = child.wait().await;
        server.stop().await;
        result
    }
    async fn close(&mut self) -> Result<()> {
        self.closed = true;
        Ok(())
    }
}
fn startup_args(request: &RunRequest, mcp: &Value, model: Option<&str>) -> Vec<String> {
    let builtins = if request.allow_web_search {
        "WebSearch,WebFetch"
    } else {
        ""
    };
    let mut allowed: Vec<String> = request
        .tools
        .iter()
        .map(|t| format!("mcp__lugus__{}", t.name))
        .collect();
    if request.allow_web_search {
        allowed.extend(["WebSearch".into(), "WebFetch".into()]);
    }
    let settings = json!({"disableAllHooks":true,"enabledPlugins":{},"autoMemoryEnabled":false,"enableAllProjectMcpServers":false});
    let mut args: Vec<String> = [
        "--print",
        "--verbose",
        "--output-format",
        "stream-json",
        "--include-partial-messages",
        "--no-session-persistence",
        "--restricted",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-chrome",
        "--permission-mode",
        "dontAsk",
        "--permission-prompts",
        "none",
        "--tools",
        builtins,
        "--allowedTools",
        &allowed.join(","),
        "--settings",
        &settings.to_string(),
        "--mcp-config",
        &mcp.to_string(),
        "--system-prompt",
        &request.instructions,
    ]
    .into_iter()
    .map(String::from)
    .collect();
    if let Some(model) = model {
        args.extend(["--model".into(), model.into()]);
    }
    args
}
async fn drive(
    mut stdout: ChildStdout,
    server: &mut McpServer,
    request: &RunRequest,
    tools: &dyn ToolExecutor,
    events: &mpsc::Sender<RuntimeEvent>,
) -> Result<RunReport> {
    let mut buffer = Vec::new();
    let mut scanned = 0;
    let mut chunk = [0u8; 8192];
    let mut text = String::new();
    let mut tool_calls = 0usize;
    let mut seen_ids = HashSet::new();
    loop {
        tokio::select! {
            read = stdout.read(&mut chunk) => {
                let count = read.map_err(process_error)?;
                if count == 0 { return Err(Error::UnexpectedEof); }
                buffer.extend_from_slice(&chunk[..count]);
                while let Some(offset) = buffer[scanned..].iter().position(|byte| *byte == b'\n') {
                    let end = scanned + offset;
                    if end > MAX_FRAME { return Err(Error::FrameTooLarge { limit: MAX_FRAME }); }
                    let frame = buffer.drain(..=end).collect::<Vec<_>>();
                    scanned = 0;
                    let utf8 = std::str::from_utf8(&frame).map_err(|_| Error::InvalidUtf8)?;
                    let message: Value = serde_json::from_str(utf8).map_err(|_| Error::MalformedJson("invalid Claude output frame".into()))?;
                    if let Some(report) = process_frame(message, &mut text, request, events).await? { return Ok(report); }
                }
                scanned = buffer.len();
                if buffer.len() > MAX_FRAME { return Err(Error::FrameTooLarge { limit: MAX_FRAME }); }
            }
            call = server.calls.recv() => {
                let Some(call) = call else { return Err(Error::Protocol("MCP server stopped".into())); };
                dispatch(call, request, tools, events, &mut tool_calls, &mut seen_ids).await?;
            }
        }
    }
}
async fn process_frame(
    message: Value,
    text: &mut String,
    request: &RunRequest,
    events: &mpsc::Sender<RuntimeEvent>,
) -> Result<Option<RunReport>> {
    let kind = message
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("Claude frame lacks type".into()))?;
    if message
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|e| matches!(e, "authentication_failed" | "authentication_error"))
    {
        return Err(Error::AuthenticationRequired);
    }
    match kind {
        "stream_event" => {
            if message.pointer("/event/delta/type").and_then(Value::as_str) == Some("text_delta") {
                let delta = message
                    .pointer("/event/delta/text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::Protocol("Claude text delta is invalid".into()))?;
                if text.len().saturating_add(delta.len()) > MAX_TEXT {
                    return Err(Error::FrameTooLarge { limit: MAX_TEXT });
                }
                text.push_str(delta);
                emit(events, RuntimeEvent::TextDelta { text: delta.into() }).await?;
            }
        }
        "result" => {
            if message.get("is_error").and_then(Value::as_bool) != Some(false)
                || message.get("subtype").and_then(Value::as_str) != Some("success")
            {
                return Err(Error::Process(
                    "Claude run failed; check CLI authentication and configuration".into(),
                ));
            }
            let result = message
                .get("result")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Protocol("Claude result lacks text".into()))?;
            if result.len() > MAX_TEXT {
                return Err(Error::FrameTooLarge { limit: MAX_TEXT });
            }
            if text.is_empty() && !result.is_empty() {
                emit(
                    events,
                    RuntimeEvent::TextDelta {
                        text: result.into(),
                    },
                )
                .await?;
            }
            if let Some(usage) = message.get("usage") {
                let input_tokens = [
                    "input_tokens",
                    "cache_creation_input_tokens",
                    "cache_read_input_tokens",
                ]
                .iter()
                .filter_map(|key| usage.get(key).and_then(Value::as_u64))
                .fold(0u64, u64::saturating_add);
                let output_tokens = usage
                    .get("output_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                emit(
                    events,
                    RuntimeEvent::Usage {
                        input_tokens,
                        output_tokens,
                    },
                )
                .await?;
            }
            return Ok(Some(RunReport {
                run_id: request.run_id.clone(),
                outcome: RunOutcome::Completed,
                final_text: result.into(),
            }));
        }
        "system" | "assistant" | "user" | "rate_limit_event" | "tool_progress"
        | "tool_use_summary" | "auth_status" => {}
        _ => return Err(Error::Protocol("Unsupported Claude output frame".into())),
    }
    Ok(None)
}
async fn dispatch(
    call: Invocation,
    request: &RunRequest,
    tools: &dyn ToolExecutor,
    events: &mpsc::Sender<RuntimeEvent>,
    count: &mut usize,
    seen: &mut HashSet<String>,
) -> Result<()> {
    let call_id = call.id.to_string();
    let result = if *count >= request.limits.max_tool_calls {
        ToolResult {
            success: false,
            content: "Tool call limit reached".into(),
        }
    } else if !seen.insert(call_id.clone()) {
        ToolResult {
            success: false,
            content: "Duplicate tool call ID".into(),
        }
    } else {
        *count += 1;
        emit(
            events,
            RuntimeEvent::ToolStarted {
                call_id: call_id.clone(),
                name: call.name.clone(),
            },
        )
        .await?;
        let mut result = tools
            .execute(ToolCall {
                run_id: request.run_id.clone(),
                call_id: call_id.clone(),
                name: call.name,
                arguments: call.arguments,
            })
            .await;
        if result.content.len() > request.limits.max_tool_result_bytes {
            result = ToolResult {
                success: false,
                content: "Tool result exceeds byte limit".into(),
            };
        }
        emit(
            events,
            RuntimeEvent::ToolFinished {
                call_id,
                success: result.success,
            },
        )
        .await?;
        result
    };
    // Even diagnostic text respects a very small caller-specified result cap.
    let mut content = result.content;
    if content.len() > request.limits.max_tool_result_bytes {
        let mut end = request.limits.max_tool_result_bytes;
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        content.truncate(end);
    }
    let _ = call
        .reply
        .send(json!({"content":[{"type":"text","text":content}],"isError":!result.success}));
    Ok(())
}
async fn emit(events: &mpsc::Sender<RuntimeEvent>, event: RuntimeEvent) -> Result<()> {
    tokio::time::timeout(EVENT_TIMEOUT, events.send(event))
        .await
        .map_err(|_| Error::EventConsumerSlow)?
        .map_err(|_| Error::EventConsumerDisconnected)
}
async fn cancelled_signal(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
fn cancelled(request: &RunRequest) -> RunReport {
    RunReport {
        run_id: request.run_id.clone(),
        outcome: RunOutcome::Cancelled,
        final_text: String::new(),
    }
}
fn process_error(error: std::io::Error) -> Error {
    Error::Process(error.to_string())
}
fn missing_pipe() -> Error {
    Error::Process("Claude process pipe unavailable".into())
}
