//! Thin CLI composition around the shared conversation host. No conversation storage here.
use super::{AgentResult, FixtureCommand, failure};
use crate::{CliResult, read_json, value};
use lugus_agent::{runtime::*, tools::ToolExecutor};
use lugus_app::{conversations::*, *};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    sync::Arc,
};
use tokio::sync::{mpsc, watch};

#[derive(Clone, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum Fixture {
    Research {
        workflow: super::BindingWorkflow,
    },
    Continue {
        expected_message_ids: Vec<String>,
        expected_dataset_id: String,
    },
}
impl Fixture {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Research { workflow } => {
                for id in [
                    &workflow.company_instance_id,
                    &workflow.market_instance_id,
                    &workflow.native_namespace,
                ] {
                    validate_id(id)?;
                }
                if workflow.input.trim().is_empty()
                    || workflow.input.len() > 16384
                    || workflow.page_size == 0
                    || workflow.page_size > 100000
                    || workflow.start > workflow.end
                {
                    return Err(invalid());
                }
            }
            Self::Continue {
                expected_message_ids,
                expected_dataset_id,
            } => {
                if expected_message_ids.len() > 64 {
                    return Err(invalid());
                }
                for id in expected_message_ids {
                    validate_id(id)?;
                }
                validate_id(expected_dataset_id)?;
            }
        }
        Ok(())
    }
}
struct Factory(Fixture);
#[async_trait::async_trait]
impl RuntimeFactory for Factory {
    async fn create(&self) -> Result<Box<dyn AgentRuntime>> {
        Ok(Box::new(Runtime(self.0.clone())))
    }
}
struct Runtime(Fixture);
#[async_trait::async_trait]
impl AgentRuntime for Runtime {
    async fn run(
        &mut self,
        request: RunRequest,
        tools: &dyn ToolExecutor,
        events: mpsc::Sender<RuntimeEvent>,
        _cancel: watch::Receiver<bool>,
    ) -> AgentResult<RunReport> {
        validate_request(&request)?;
        if !matches!(
            request.subject,
            lugus_agent::RunSubject::Conversation { .. }
        ) {
            return Err(failure(
                "conversation fixture requires a conversation subject",
            ));
        }
        events
            .send(RuntimeEvent::TextDelta {
                text: "Inspecting the selected research evidence.".into(),
            })
            .await
            .map_err(|_| failure("event stream closed"))?;
        let output = match &self.0 {
            Fixture::Research { workflow } => {
                let receipts = FixtureCommand::Binding(workflow.clone())
                    .sequence(&request, tools, &events)
                    .await?;
                json!({"subject_kind":"conversation","answer":"Research evidence stored; price chart view accepted.","receipts":{
                    "company_dataset":{"id":receipts["company_dataset"]["id"]},
                    "instrument_fetch":{"id":receipts["instrument_fetch"]["id"]},
                    "binding":{"id":receipts["binding"]["id"]},
                    "fetch":{"id":receipts["fetch"]["id"],"command":{"query":{"instrument":receipts["fetch"]["command"]["query"]["instrument"]}}},
                    "dataset":{"id":receipts["dataset"]["id"]},"view":{"id":receipts["view"]["id"]}
                }})
            }
            Fixture::Continue {
                expected_message_ids,
                expected_dataset_id,
            } => {
                let context: Value = serde_json::from_str(&request.prompt)
                    .map_err(|_| failure("invalid conversation context"))?;
                let exchanges = context["exchanges"]
                    .as_array()
                    .ok_or_else(|| failure("missing prior exchanges"))?;
                let messages: Vec<&Value> = exchanges
                    .iter()
                    .filter_map(|e| e["messages"].as_array())
                    .flatten()
                    .collect();
                let ids: Vec<&str> = messages.iter().filter_map(|m| m["id"].as_str()).collect();
                if ids
                    != expected_message_ids
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                    || !messages.iter().any(|m| {
                        m["role"] == "assistant"
                            && m["text"].as_str().is_some_and(|s| !s.is_empty())
                    })
                {
                    return Err(failure("prior messages differ from expected context"));
                }
                let reference = context["references"]
                    .as_array()
                    .and_then(|refs| {
                        refs.iter().find(|r| {
                            r["reference"]["kind"] == "dataset"
                                && r["reference"]["id"] == *expected_dataset_id
                        })
                    })
                    .ok_or_else(|| failure("expected selected dataset absent"))?;
                let frozen: Value = serde_json::from_str(
                    reference["serialized"]
                        .as_str()
                        .ok_or_else(|| failure("missing frozen dataset"))?,
                )
                .map_err(|_| failure("invalid frozen dataset"))?;
                if frozen["page"]["header"]["id"] != *expected_dataset_id
                    || frozen["page"]["rows"].as_array().is_none_or(Vec::is_empty)
                {
                    return Err(failure("selected dataset evidence missing"));
                }
                json!({"subject_kind":"conversation","answer":"Prior messages and selected dataset verified in a fresh runtime.","message_ids":ids,"dataset_id":expected_dataset_id})
            }
        };
        Ok(RunReport {
            run_id: request.run_id,
            outcome: RunOutcome::Completed,
            final_text: output.to_string(),
        })
    }
    async fn close(&mut self) -> AgentResult<()> {
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    request_id: String,
    title: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Layout {
    expected_revision: u64,
    mutation: WorkspaceMutation,
}
#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Control {
    Cancel {
        conversation_id: String,
        run_id: String,
    },
}
fn invalid() -> AppError {
    AppError::new(
        ErrorKind::InvalidInput,
        "invalid conversation command",
        false,
    )
}
fn emit(v: Value) -> CliResult<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, &v)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}
// A dedicated OS thread is deliberately not joined: idle stdin cannot keep the Tokio
// runtime alive after host cleanup. At most one bounded line and one pending control.
fn controls() -> mpsc::Receiver<Result<Control>> {
    let (sender, receiver) = mpsc::channel(1);
    std::thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        loop {
            let mut bytes = Vec::new();
            let read = std::io::Read::take(&mut input, 4097).read_until(b'\n', &mut bytes);
            match read {
                Ok(0) => break,
                Ok(_) if bytes.len() <= 4096 => {
                    let command = serde_json::from_slice(&bytes).map_err(|_| invalid());
                    if sender.blocking_send(command).is_err() {
                        break;
                    }
                }
                _ => {
                    let _ = sender.blocking_send(Err(AppError::new(
                        ErrorKind::ResourceLimit,
                        "control line exceeds limit",
                        false,
                    )));
                    break;
                }
            }
        }
    });
    receiver
}
async fn execute(
    app: Application,
    request: SendMessageRequest,
    fixture: Fixture,
) -> CliResult<Value> {
    let host = match ConversationHost::start(
        app.clone(),
        Arc::new(Factory(fixture)),
        ConversationOptions::default(),
    )
    .await
    {
        Ok(host) => host,
        Err(error) => {
            app.shutdown().await?;
            return Err(error.into());
        }
    };
    let result = async {
        let receipt = host.send(request).await?;
        emit(json!({"event":"admitted","run":receipt}))?;
        let mut controls = controls();
        let mut open = true;
        let wait = host.wait(&receipt.conversation_id, &receipt.id);
        tokio::pin!(wait);
        let terminal = loop {
            tokio::select! {
                result = &mut wait => break result?,
                control = controls.recv(), if open => match control {
                    None => open = false,
                    Some(control) => {
                        let result = match control {
                            Ok(Control::Cancel { conversation_id, run_id })
                                if conversation_id == receipt.conversation_id && run_id == receipt.id =>
                            {
                                host.cancel(&conversation_id, &run_id).await.map(|_| ())
                            }
                            Ok(_) => Err(AppError::new(ErrorKind::ScopeMismatch, "control must target this owning session", false)),
                            Err(error) => Err(error),
                        };
                        if let Err(error) = result {
                            emit(json!({"event":"control_error","error":error}))?;
                        }
                    }
                }
            }
        };
        Ok(json!({"event":"terminal","run":terminal}))
    }.await;
    host.shutdown().await?;
    result
}
fn page(offset: &str, limit: &str) -> CliResult<PageRequest> {
    Ok(PageRequest {
        offset: offset.parse()?,
        limit: limit.parse()?,
    })
}
async fn offline(app: &Application, command: &str, args: &[&str]) -> CliResult<Value> {
    match (command, args) {
        ("create", [file]) => {
            let c: Create = read_json(file).await?;
            value(app.create_conversation(&c.request_id, &c.title).await?)
        }
        ("list", [offset, limit]) => value(app.conversations(page(offset, limit)?).await?),
        ("show", [c]) => value(app.conversation(c).await?),
        ("workspace", [c]) => value(app.conversation_workspace(c).await?),
        ("layout", [c, file]) => {
            let layout: Layout = read_json(file).await?;
            value(
                app.mutate_conversation_workspace(c, layout.expected_revision, layout.mutation)
                    .await?,
            )
        }
        ("messages", [c, offset, limit]) => {
            value(app.conversation_messages(c, page(offset, limit)?).await?)
        }
        ("runs", [c, offset, limit]) => {
            value(app.conversation_runs(c, page(offset, limit)?).await?)
        }
        ("status", [c, r]) => value(app.conversation_run(c, r).await?),
        ("context", [c, r]) => value(app.conversation_run(c, r).await?.input),
        ("activity", [c, r, offset, limit]) => value(
            app.conversation_activity(c, r, page(offset, limit)?)
                .await?,
        ),
        ("tools", [c, r, offset, limit]) => value(
            app.conversation_tool_records(c, r, page(offset, limit)?)
                .await?,
        ),
        ("dataset", [c, dataset, offset, limit]) => {
            let c = app.conversation(c).await?;
            let scope = app.scope(&c.workspace_id, "conversation-read", None)?;
            value(
                app.read_dataset(&scope, dataset, page(offset, limit)?)
                    .await?,
            )
        }
        ("view", [c, view]) => {
            let c = app.conversation(c).await?;
            let scope = app.scope(&c.workspace_id, "conversation-read", None)?;
            value(app.read_view(&scope, view).await?)
        }
        _ => Err(invalid().into()),
    }
}
pub async fn run(args: &[&str]) -> CliResult<Value> {
    let [command, config, tail @ ..] = args else {
        return Err(invalid().into());
    };
    // Parse strict bounded external inputs before opening storage or acquiring ownership.
    if let ("send" | "continue", [request, fixture]) = (*command, tail) {
        let request: SendMessageRequest = read_json(request).await?;
        let fixture: Fixture = read_json(fixture).await?;
        fixture.validate()?;
        if *command == "continue" && !matches!(fixture, Fixture::Continue { .. }) {
            return Err(invalid().into());
        }
        let offline = matches!(fixture, Fixture::Continue { .. });
        let app = ApplicationConfig::load(config).await?.open(offline).await?;
        return execute(app, request, fixture).await;
    }
    let app = ApplicationConfig::load(config).await?.open(true).await?;
    if *command == "recover" && tail.is_empty() {
        let host = match ConversationHost::recover(app.clone()).await {
            Ok(host) => host,
            Err(error) => {
                app.shutdown().await?;
                return Err(error.into());
            }
        };
        host.shutdown().await?;
        return Ok(json!({"recovered":true}));
    }
    let result = offline(&app, command, tail).await;
    app.shutdown().await?;
    result
}
