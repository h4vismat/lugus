use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;

use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use super::events::{
    agent_text, last_agent_text, response_id_at, string_field, turn_error_message, validate_active,
    validate_completion,
};
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
            &["app-server".into()],
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
        let id = self.take_request_id()?;
        self.process
            .send(&json!({"id": id, "method": method, "params": params}))
            .await?;

        loop {
            match classify_message(self.process.receive().await?)? {
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
                    self.process.send(&method_not_found_response(id)).await?;
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
    ) -> Result<RunReport> {
        if self.account_status == AccountStatus::LoginRequired {
            return Err(Error::AuthenticationRequired);
        }

        let thread = self
            .rpc("thread/start", self.thread_start_params(request))
            .await?;
        let thread_id = response_id_at(&thread, &["thread", "id"], "thread/start")?;

        let turn = self
            .rpc(
                "turn/start",
                json!({
                    "threadId": thread_id,
                    "input": [{"type": "text", "text": request.prompt}],
                }),
            )
            .await?;
        let turn_id = response_id_at(&turn, &["turn", "id"], "turn/start")?;
        let _ = events
            .send(RuntimeEvent::Started {
                run_id: request.run_id.clone(),
            })
            .await;

        self.drive_turn(request, tools, events, cancel, &thread_id, &turn_id)
            .await
    }

    fn thread_start_params(&self, request: &RunRequest) -> Value {
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
        });
        if let Some(model) = &self.model {
            params["model"] = json!(model);
        }
        if let Some(provider) = &self.model_provider {
            params["modelProvider"] = json!(provider);
        }
        params
    }

    async fn drive_turn(
        &mut self,
        request: &RunRequest,
        tools: &dyn ToolExecutor,
        events: &mpsc::Sender<RuntimeEvent>,
        cancel: &mut watch::Receiver<bool>,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<RunReport> {
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
                changed = cancel.changed() => {
                    if changed.is_ok() && *cancel.borrow() {
                        return Err(Error::Cancelled);
                    }
                    continue;
                }
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
                        self.process.send(&tool_response(id, &result)).await?;
                        return Err(Error::Tool("tool call limit exceeded".into()));
                    }
                    tool_calls += 1;

                    if let Some(namespace) = params.get("namespace").and_then(Value::as_str) {
                        let result = bounded_failure(
                            &format!("unknown tool: {namespace}/{name}"),
                            request.limits.max_tool_result_bytes,
                        );
                        self.process.send(&tool_response(id, &result)).await?;
                        continue;
                    }

                    let _ = events
                        .send(RuntimeEvent::ToolStarted {
                            call_id: call_id.into(),
                            name: name.into(),
                        })
                        .await;

                    let mut result = if registered.contains(name) {
                        tools
                            .execute(ToolCall {
                                run_id: request.run_id.clone(),
                                call_id: call_id.into(),
                                name: name.into(),
                                arguments: params["arguments"].clone(),
                            })
                            .await
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
                    self.process.send(&tool_response(id, &result)).await?;
                    let _ = events
                        .send(RuntimeEvent::ToolFinished {
                            call_id: call_id.into(),
                            success: result.success,
                        })
                        .await;
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
                    let _ = events
                        .send(RuntimeEvent::TextDelta { text: delta.into() })
                        .await;
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
            return Err(Error::Cancelled);
        }
        match tokio::time::timeout(
            request.limits.timeout,
            self.run_inner(&request, tools, &events, &mut cancel),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(Error::Timeout),
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
