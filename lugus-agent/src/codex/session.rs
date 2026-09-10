use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use super::events::{
    agent_text, last_agent_text, response_id_at, string_field, turn_error_message, validate_active,
    validate_completion,
};
use super::policy::{mcp_server_ids, startup_args, unattended_config};
use super::process::{CodexProcess, TransportLimits};
use super::protocol::{
    NotificationMethod, RequestMethod, ResponseOutcome, WireMessage, classify_message,
    method_not_found_response, tool_response,
};
use super::version::probe_supported_version;
use crate::{
    AgentRuntime, Error, Result, RunOutcome, RunReport, RunRequest, RuntimeEvent, ToolCall,
    ToolExecutor, ToolResult, validate_request,
};

const EVENT_DELIVERY_TIMEOUT: Duration = Duration::from_millis(100);
const INTERRUPT_GRACE: Duration = Duration::from_secs(2);
const RPC_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
struct TurnScope<'a> {
    request: &'a RunRequest,
    thread_id: &'a str,
    turn_id: &'a str,
    deadline: tokio::time::Instant,
}

struct ActiveTurn<'a> {
    tools: &'a dyn ToolExecutor,
    events: &'a mpsc::Sender<RuntimeEvent>,
    cancel: &'a mut watch::Receiver<bool>,
    scope: TurnScope<'a>,
}

#[derive(Debug, Clone)]
pub struct CodexConfig {
    pub executable: PathBuf,
    pub workspace: PathBuf,
    pub model: Option<String>,
    pub model_provider: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountStatus {
    Ready,
    LoginRequired,
}

pub struct CodexRuntime {
    process: CodexProcess,
    workspace: PathBuf,
    model: Option<String>,
    model_provider: Option<String>,
    next_request_id: u64,
    account_status: AccountStatus,
}

impl fmt::Debug for CodexRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodexRuntime")
            .field("workspace", &self.workspace)
            .field("model", &self.model)
            .field("model_provider", &self.model_provider)
            .field("next_request_id", &self.next_request_id)
            .field("account_status", &self.account_status)
            .finish_non_exhaustive()
    }
}

impl CodexRuntime {
    pub async fn connect(config: CodexConfig) -> Result<Self> {
        validate_config(&config)?;
        probe_supported_version(&config.executable, &config.workspace).await?;

        let process = CodexProcess::spawn(
            &config.executable,
            &startup_args(config.model.as_deref(), config.model_provider.as_deref()),
            &config.workspace,
            TransportLimits::default(),
        )?;
        let mut runtime = Self {
            process,
            workspace: config.workspace,
            model: config.model,
            model_provider: config.model_provider,
            next_request_id: 1,
            account_status: AccountStatus::LoginRequired,
        };

        if let Err(error) = runtime.initialize().await {
            let _ = runtime.process.close().await;
            return Err(error);
        }
        Ok(runtime)
    }

    pub async fn account_status(&mut self) -> Result<AccountStatus> {
        Ok(self.account_status)
    }

    async fn initialize(&mut self) -> Result<()> {
        self.rpc(
            "initialize",
            json!({
                "clientInfo": {"name": "lugus", "version": "0.1.0"},
                "capabilities": {"experimentalApi": true},
            }),
        )
        .await?;
        self.process.send(&json!({"method": "initialized"})).await?;

        let account = self.rpc("account/read", json!({})).await?;
        self.account_status = parse_account_status(&account)?;
        Ok(())
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + RPC_TIMEOUT;
        let id = self.take_request_id()?;
        let request = json!({"id": id, "method": method, "params": params});
        tokio::time::timeout_at(deadline, self.process.send(&request))
            .await
            .map_err(|_| Error::Timeout)??;

        loop {
            let message = tokio::time::timeout_at(deadline, self.process.receive())
                .await
                .map_err(|_| Error::Timeout)??;
            match classify_message(message)? {
                WireMessage::Response {
                    id: response_id,
                    outcome,
                } => {
                    if response_id != json!(id) {
                        return Err(Error::Protocol(format!(
                            "response id {response_id} does not match request id {id}"
                        )));
                    }
                    return match outcome {
                        ResponseOutcome::Result(result) => Ok(result),
                        ResponseOutcome::Error(error) => Err(rpc_error(method, &error)),
                    };
                }
                WireMessage::Request { id, .. } => {
                    let response = method_not_found_response(id);
                    tokio::time::timeout_at(deadline, self.process.send(&response))
                        .await
                        .map_err(|_| Error::Timeout)??;
                }
                WireMessage::Notification { .. } => {}
            }
        }
    }

    fn take_request_id(&mut self) -> Result<u64> {
        let id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or_else(|| Error::Protocol("client request id overflow".into()))?;
        Ok(id)
    }

    async fn run_inner(
        &mut self,
        request: &RunRequest,
        tools: &dyn ToolExecutor,
        events: &mpsc::Sender<RuntimeEvent>,
        cancel: &mut watch::Receiver<bool>,
        deadline: tokio::time::Instant,
    ) -> Result<RunReport> {
        if *cancel.borrow() {
            return self.cancel_before_turn(request).await;
        }
        if self.account_status == AccountStatus::LoginRequired {
            return Err(Error::AuthenticationRequired);
        }

        let mcp_server_ids = tokio::select! {
            biased;
            () = wait_for_cancellation(cancel) => return self.cancel_before_turn(request).await,
            ids = self.current_mcp_server_ids() => ids?,
            () = tokio::time::sleep_until(deadline) => return Err(Error::Timeout),
        };

        let thread_start_params = self.thread_start_params(request, &mcp_server_ids);
        let thread = tokio::select! {
            biased;
            () = wait_for_cancellation(cancel) => return self.cancel_before_turn(request).await,
            thread = self.rpc("thread/start", thread_start_params) => thread?,
            () = tokio::time::sleep_until(deadline) => return Err(Error::Timeout),
        };
        let thread_id = response_id_at(&thread, &["thread", "id"], "thread/start")?;

        let turn_start_params = json!({
            "threadId": thread_id,
            "input": [{"type": "text", "text": request.prompt}],
        });
        let turn = tokio::select! {
            biased;
            () = wait_for_cancellation(cancel) => return self.cancel_before_turn(request).await,
            turn = self.rpc("turn/start", turn_start_params) => turn?,
            () = tokio::time::sleep_until(deadline) => return Err(Error::Timeout),
        };
        let turn_id = response_id_at(&turn, &["turn", "id"], "turn/start")?;
        if emit_event(
            events,
            RuntimeEvent::Started {
                run_id: request.run_id.clone(),
            },
            deadline,
            cancel,
        )
        .await?
        {
            return self
                .interrupt_and_report(request, &thread_id, &turn_id)
                .await;
        }

        self.drive_turn(ActiveTurn {
            tools,
            events,
            cancel,
            scope: TurnScope {
                request,
                thread_id: &thread_id,
                turn_id: &turn_id,
                deadline,
            },
        })
        .await
    }

    async fn current_mcp_server_ids(&mut self) -> Result<Vec<String>> {
        let config = self
            .rpc(
                "config/read",
                json!({"cwd": self.workspace, "includeLayers": false}),
            )
            .await?;
        mcp_server_ids(&config)
    }

    async fn cancel_before_turn(&mut self, request: &RunRequest) -> Result<RunReport> {
        let _ = self.process.abort().await;
        Ok(cancelled_report(request))
    }

    fn thread_start_params(&self, request: &RunRequest, mcp_server_ids: &[String]) -> Value {
        let dynamic_tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                })
            })
            .collect();
        let mut params = json!({
            "cwd": self.workspace,
            "ephemeral": true,
            "baseInstructions": request.instructions,
            "developerInstructions": request.context,
            "dynamicTools": dynamic_tools,
            "approvalPolicy": "never",
            "sandbox": "read-only",
            "config": unattended_config(mcp_server_ids),
        });
        if let Some(model) = &self.model {
            params["model"] = json!(model);
        }
        if let Some(provider) = &self.model_provider {
            params["modelProvider"] = json!(provider);
        }
        params
    }

    async fn drive_turn(&mut self, active: ActiveTurn<'_>) -> Result<RunReport> {
        let ActiveTurn {
            tools,
            events,
            cancel,
            scope,
        } = active;
        let TurnScope {
            request,
            thread_id,
            turn_id,
            deadline,
        } = scope;
        let registered: HashSet<&str> = request
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect();
        let mut seen_calls = HashSet::new();
        let mut tool_calls = 0_usize;
        let mut streamed_text = HashMap::<String, String>::new();
        let mut active_message_id: Option<String> = None;
        let mut final_text = None;

        loop {
            let message = tokio::select! {
                message = self.process.receive() => message?,
                () = wait_for_cancellation(cancel) => return self.interrupt_and_report(request, thread_id, turn_id).await,
                () = tokio::time::sleep_until(deadline) => return Err(Error::Timeout),
            };
            match classify_message(message)? {
                WireMessage::Request {
                    id,
                    method: RequestMethod::DynamicToolCall,
                    params,
                } => {
                    validate_active(&params, thread_id, turn_id, "dynamic tool call")?;
                    let call_id = string_field(&params, "callId", "dynamic tool call")?;
                    if !seen_calls.insert(call_id.to_owned()) {
                        return Err(Error::Protocol(format!(
                            "duplicate tool call id: {call_id}"
                        )));
                    }
                    let name = string_field(&params, "tool", "dynamic tool call")?;
                    if tool_calls >= request.limits.max_tool_calls {
                        let result = bounded_failure(
                            "tool call limit exceeded",
                            request.limits.max_tool_result_bytes,
                        );
                        self.send_turn_response(id, &result, cancel, scope).await?;
                        return Err(Error::Tool("tool call limit exceeded".into()));
                    }
                    tool_calls += 1;

                    if let Some(namespace) = params.get("namespace").and_then(Value::as_str) {
                        let result = bounded_failure(
                            &format!("unknown tool: {namespace}/{name}"),
                            request.limits.max_tool_result_bytes,
                        );
                        self.send_turn_response(id, &result, cancel, scope).await?;
                        continue;
                    }

                    if emit_event(
                        events,
                        RuntimeEvent::ToolStarted {
                            call_id: call_id.into(),
                            name: name.into(),
                        },
                        deadline,
                        cancel,
                    )
                    .await?
                    {
                        return self.interrupt_and_report(request, thread_id, turn_id).await;
                    }

                    let mut result = if registered.contains(name) {
                        tokio::select! {
                            result = tools.execute(ToolCall {
                                run_id: request.run_id.clone(),
                                call_id: call_id.into(),
                                name: name.into(),
                                arguments: params["arguments"].clone(),
                            }) => result,
                            () = wait_for_cancellation(cancel) => return self.interrupt_and_report(request, thread_id, turn_id).await,
                            () = tokio::time::sleep_until(deadline) => return Err(Error::Timeout),
                        }
                    } else {
                        bounded_failure(
                            &format!("unknown tool: {name}"),
                            request.limits.max_tool_result_bytes,
                        )
                    };
                    if result.content.len() > request.limits.max_tool_result_bytes {
                        result = bounded_failure(
                            "tool result exceeds configured byte limit",
                            request.limits.max_tool_result_bytes,
                        );
                    }
                    self.send_turn_response(id, &result, cancel, scope).await?;
                    if emit_event(
                        events,
                        RuntimeEvent::ToolFinished {
                            call_id: call_id.into(),
                            success: result.success,
                        },
                        deadline,
                        cancel,
                    )
                    .await?
                    {
                        return self.interrupt_and_report(request, thread_id, turn_id).await;
                    }
                }
                WireMessage::Request {
                    id,
                    method: RequestMethod::Approval,
                    ..
                } => {
                    self.process
                        .send(&json!({"id": id, "result": {"decision": "cancel"}}))
                        .await?;
                    return Err(Error::NeedsAttention(
                        "Codex requested approval in an unattended run".into(),
                    ));
                }
                WireMessage::Request {
                    id,
                    method: RequestMethod::McpElicitation,
                    ..
                } => {
                    self.process
                        .send(&json!({"id": id, "result": {"action": "cancel"}}))
                        .await?;
                    return Err(Error::NeedsAttention(
                        "Codex requested MCP user input in an unattended run".into(),
                    ));
                }
                WireMessage::Request {
                    id,
                    method: RequestMethod::HumanInput,
                    ..
                } => {
                    self.process.send(&method_not_found_response(id)).await?;
                    return Err(Error::NeedsAttention(
                        "Codex requested human input in an unattended run".into(),
                    ));
                }
                WireMessage::Request {
                    id,
                    method: RequestMethod::Unknown(_),
                    ..
                } => self.process.send(&method_not_found_response(id)).await?,
                WireMessage::Response { .. } => {
                    return Err(Error::Protocol(
                        "unexpected response while turn is active".into(),
                    ));
                }
                WireMessage::Notification {
                    method: NotificationMethod::TurnCompleted,
                    params: Some(params),
                } => {
                    validate_completion(&params, thread_id, turn_id)?;
                    let turn = params["turn"].as_object().expect("protocol validated turn");
                    if let Some(text) = last_agent_text(turn.get("items")) {
                        final_text = Some(text.to_owned());
                    }
                    let outcome = match turn.get("status").and_then(Value::as_str) {
                        Some("completed") => RunOutcome::Completed,
                        Some("interrupted") => RunOutcome::Cancelled,
                        Some("failed") => {
                            return Err(Error::NeedsAttention(turn_error_message(
                                turn.get("error"),
                            )));
                        }
                        _ => {
                            return Err(Error::Protocol(
                                "turn completion has a non-terminal status".into(),
                            ));
                        }
                    };
                    return Ok(RunReport {
                        run_id: request.run_id.clone(),
                        outcome,
                        final_text: final_text.unwrap_or_else(|| {
                            active_message_id
                                .and_then(|item_id| streamed_text.remove(&item_id))
                                .unwrap_or_default()
                        }),
                    });
                }
                WireMessage::Notification {
                    method: NotificationMethod::Unknown(method),
                    params: Some(params),
                } if method == "item/agentMessage/delta" => {
                    validate_active(&params, thread_id, turn_id, "agent message delta")?;
                    let delta = string_field(&params, "delta", "agent message delta")?;
                    let item_id = string_field(&params, "itemId", "agent message delta")?;
                    streamed_text
                        .entry(item_id.into())
                        .or_default()
                        .push_str(delta);
                    active_message_id = Some(item_id.into());
                    if emit_event(
                        events,
                        RuntimeEvent::TextDelta { text: delta.into() },
                        deadline,
                        cancel,
                    )
                    .await?
                    {
                        return self.interrupt_and_report(request, thread_id, turn_id).await;
                    }
                }
                WireMessage::Notification {
                    method: NotificationMethod::Unknown(method),
                    params: Some(params),
                } if method == "item/completed" => {
                    validate_active(&params, thread_id, turn_id, "completed item")?;
                    if let Some(text) = agent_text(params.get("item")) {
                        final_text = Some(text.to_owned());
                    }
                }
                WireMessage::Notification { .. } => {}
            }
        }
    }

    async fn send_turn_response(
        &mut self,
        id: Value,
        result: &ToolResult,
        cancel: &mut watch::Receiver<bool>,
        scope: TurnScope<'_>,
    ) -> Result<()> {
        let response = tool_response(id, result);
        tokio::select! {
            sent = self.process.send(&response) => sent,
            () = wait_for_cancellation(cancel) => {
                let _ = self.interrupt_and_report(scope.request, scope.thread_id, scope.turn_id).await;
                Err(Error::Cancelled)
            }
            () = tokio::time::sleep_until(scope.deadline) => Err(Error::Timeout),
        }
    }

    async fn interrupt_and_report(
        &mut self,
        request: &RunRequest,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<RunReport> {
        let id = self.take_request_id()?;
        let interrupt = json!({
            "id": id,
            "method": "turn/interrupt",
            "params": {"threadId": thread_id, "turnId": turn_id},
        });

        let completed = async {
            self.process.send(&interrupt).await?;
            loop {
                match classify_message(self.process.receive().await?)? {
                    WireMessage::Notification {
                        method: NotificationMethod::TurnCompleted,
                        params: Some(params),
                    } => {
                        validate_completion(&params, thread_id, turn_id)?;
                        break Ok::<(), Error>(());
                    }
                    WireMessage::Request { id, .. } => {
                        self.process.send(&method_not_found_response(id)).await?;
                    }
                    WireMessage::Response { .. } | WireMessage::Notification { .. } => {}
                }
            }
        };
        let _ = tokio::time::timeout(INTERRUPT_GRACE, completed).await;
        let _ = self.process.abort().await;

        Ok(cancelled_report(request))
    }
}

#[async_trait::async_trait]
impl AgentRuntime for CodexRuntime {
    async fn run(
        &mut self,
        request: RunRequest,
        tools: &dyn ToolExecutor,
        events: mpsc::Sender<RuntimeEvent>,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<RunReport> {
        validate_request(&request)?;
        if *cancel.borrow() {
            let _ = self.process.abort().await;
            return Ok(cancelled_report(&request));
        }
        let deadline = tokio::time::Instant::now() + request.limits.timeout;
        let result = match tokio::time::timeout(
            request.limits.timeout,
            self.run_inner(&request, tools, &events, &mut cancel, deadline),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(Error::Timeout),
        };
        match result {
            Err(Error::Cancelled) => {
                let _ = self.process.close().await;
                Ok(cancelled_report(&request))
            }
            Err(error) => {
                let _ = self.process.close().await;
                Err(error)
            }
            Ok(report) if report.outcome == RunOutcome::Cancelled => {
                let _ = self.process.close().await;
                Ok(report)
            }
            Ok(report) => Ok(report),
        }
    }

    async fn close(&mut self) -> Result<()> {
        self.process.close().await
    }
}

fn validate_config(config: &CodexConfig) -> Result<()> {
    if config.executable.as_os_str().is_empty() {
        return Err(Error::Configuration(
            "Codex executable must not be empty".into(),
        ));
    }
    if !config.workspace.is_dir() {
        return Err(Error::Configuration(
            "Codex workspace must be a directory".into(),
        ));
    }
    Ok(())
}

fn parse_account_status(value: &Value) -> Result<AccountStatus> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::Protocol("account/read returned a malformed result".into()))?;
    let requires_auth = object
        .get("requiresOpenaiAuth")
        .and_then(Value::as_bool)
        .ok_or_else(|| Error::Protocol("account/read returned a malformed result".into()))?;
    Ok(
        if requires_auth && object.get("account").is_none_or(Value::is_null) {
            AccountStatus::LoginRequired
        } else {
            AccountStatus::Ready
        },
    )
}

fn bounded_failure(message: &str, limit: usize) -> ToolResult {
    let mut end = message.len().min(limit);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    ToolResult {
        success: false,
        content: message[..end].into(),
    }
}

fn cancelled_report(request: &RunRequest) -> RunReport {
    RunReport {
        run_id: request.run_id.clone(),
        outcome: RunOutcome::Cancelled,
        final_text: String::new(),
    }
}

async fn wait_for_cancellation(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow() {
            return;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

async fn emit_event(
    events: &mpsc::Sender<RuntimeEvent>,
    event: RuntimeEvent,
    deadline: tokio::time::Instant,
    cancel: &mut watch::Receiver<bool>,
) -> Result<bool> {
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    if remaining.is_zero() {
        return Err(Error::Timeout);
    }
    tokio::select! {
        sent = tokio::time::timeout(EVENT_DELIVERY_TIMEOUT.min(remaining), events.send(event)) => match sent {
            Ok(Ok(())) => Ok(false),
            Ok(Err(_)) => Err(Error::EventConsumerDisconnected),
            Err(_) if tokio::time::Instant::now() >= deadline => Err(Error::Timeout),
            Err(_) => Err(Error::EventConsumerSlow),
        },
        () = wait_for_cancellation(cancel) => Ok(true),
        () = tokio::time::sleep_until(deadline) => Err(Error::Timeout),
    }
}

fn rpc_error(method: &str, error: &Value) -> Error {
    let code = error.get("code").and_then(Value::as_i64);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unspecified RPC error");
    match code {
        Some(code) => Error::Protocol(format!("{method} RPC error {code}: {message}")),
        None => Error::Protocol(format!("{method} RPC error: {message}")),
    }
}
