//! Host-bound, provider/model-neutral research tools.
use crate::{
    agent_contract::{check_serialized_size, decode_fetch_call, fetch_tool_specs, scope_for_call},
    *,
};
use lugus_agent::tools::{ToolCall, ToolExecutor, ToolResult, ToolSpec};
use serde::Deserialize;
use serde_json::{Value, json};

mod bindings;
mod passages;
mod price_chart;

pub struct ResearchExecutor {
    application: Application,
    scope: Scope,
    offering: Offering,
    specs: Vec<ToolSpec>,
    output_bytes: usize,
    cancellation: Option<tokio::sync::watch::Receiver<bool>>,
}
impl ResearchExecutor {
    /// The host captures one immutable turn offering. Model arguments never construct scope.
    pub fn new(application: Application, scope: Scope) -> Result<Self> {
        scope.validate()?;
        if scope.run_id.is_none() {
            return Err(invalid("agent scope requires a run ID"));
        }
        let offering = application.offering()?;
        let mut specs = fetch_tool_specs(&offering);
        specs.extend(price_chart::tool_spec(&offering));
        specs.extend(cached_tool_specs(application.limits()));
        specs.extend(bindings::tool_specs(application.limits(), &offering));
        specs.extend(passages::tool_specs(&application));
        let output_bytes = application.limits().max_output_bytes;
        Ok(Self {
            output_bytes,
            cancellation: None,
            application,
            scope,
            offering,
            specs,
        })
    }
    pub(crate) fn for_conversation(
        application: Application,
        scope: Scope,
        output_bytes: usize,
        cancellation: tokio::sync::watch::Receiver<bool>,
    ) -> Result<Self> {
        let mut executor = Self::new(application, scope)?;
        executor.output_bytes = executor.output_bytes.min(output_bytes);
        executor.cancellation = Some(cancellation);
        Ok(executor)
    }
    pub fn tool_specs(&self) -> &[ToolSpec] {
        &self.specs
    }
    /// Explicit allowlist: the analysis model cannot invoke hidden fetch or binding effects.
    pub(crate) fn restrict_to_evidence(&mut self) {
        const OFFLINE: &[&str] = &[
            "lugus_read_fetch",
            "lugus_dataset_header",
            "lugus_read_dataset",
            "lugus_read_document",
            "lugus_read_view",
            "lugus_open_view",
            "lugus_read_binding",
            "lugus_list_bindings",
            "lugus_binding_history",
            "lugus_text_header",
            "lugus_read_text",
            "lugus_read_passage",
            "lugus_resolve_passage",
        ];
        self.specs
            .retain(|spec| OFFLINE.contains(&spec.name.as_str()));
    }
    async fn dispatch(&self, call: &ToolCall) -> Result<(bool, Value)> {
        let scope = scope_for_call(&self.scope, call)?;
        let limit = self.application.limits().max_input_bytes;
        check_serialized_size(&call.arguments, limit)?;
        if !self.specs.iter().any(|s| s.name == call.name) {
            return Err(AppError::new(
                ErrorKind::Unsupported,
                "tool was not offered for this turn",
                false,
            ));
        }
        if call.name == price_chart::NAME {
            return self.price_chart_call(&scope, call).await;
        }
        if passages::NAMES.contains(&call.name.as_str()) {
            return self.passage_call(&scope, call).await;
        }
        if bindings::NAMES.contains(&call.name.as_str()) {
            return self.binding_call(&scope, call).await;
        }
        if let Some(name) = call
            .name
            .strip_prefix("lugus_")
            .filter(|n| LOCAL_NAMES.contains(n))
        {
            let mut args = call
                .arguments
                .as_object()
                .cloned()
                .ok_or_else(|| invalid("tool arguments must be an object"))?;
            if args.contains_key("operation") {
                return Err(invalid("operation is selected by tool name"));
            }
            args.insert("operation".into(), json!(name));
            let command: CachedCommand = serde_json::from_value(Value::Object(args))
                .map_err(|_| invalid("invalid cached tool arguments"))?;
            return self.cached(&scope, command).await.map(|v| (true, v));
        }
        let command = decode_fetch_call(call, limit)?;
        let receipt = self.application.submit(&scope, &self.offering, command)?;
        self.await_job(&scope, receipt).await
    }
    async fn await_job(&self, scope: &Scope, receipt: JobReceipt) -> Result<(bool, Value)> {
        let mut guard = CancelOnDrop {
            application: self.application.clone(),
            scope: scope.clone(),
            id: Some(receipt.id.clone()),
        };
        let terminal = self.application.wait(scope, &receipt.id);
        tokio::pin!(terminal);
        if let Some(mut cancel) = self.cancellation.clone() {
            tokio::select! {
                biased;
                _ = async { while !*cancel.borrow_and_update() { if cancel.changed().await.is_err() { break; } } } => {
                    self.application.cancel(scope, &receipt.id)?;
                }
                terminal = &mut terminal => {
                    let terminal = terminal?;
                    guard.id = None;
                    return Ok((terminal.state == JobState::Succeeded, self.value(terminal)?));
                }
            }
        }
        let terminal = terminal.await?;
        guard.id = None;
        Ok((terminal.state == JobState::Succeeded, self.value(terminal)?))
    }
    fn value<T: serde::Serialize>(&self, value: T) -> Result<Value> {
        check_serialized_size(&value, self.application.limits().max_output_bytes)?;
        serde_json::to_value(value).map_err(|_| invalid("could not encode tool result"))
    }
    async fn cached(&self, scope: &Scope, command: CachedCommand) -> Result<Value> {
        let app = &self.application;
        match command {
            CachedCommand::ReadFetch { fetch_id } => {
                self.value(app.read_fetch(scope, &fetch_id).await?)
            }
            CachedCommand::CreateDataset {
                fetch_id,
                projection,
            } => self.value(app.create_dataset(scope, &fetch_id, projection).await?),
            CachedCommand::DatasetHeader { dataset_id } => {
                self.value(app.dataset_header(scope, &dataset_id).await?)
            }
            CachedCommand::ReadDataset { dataset_id, page } => {
                self.value(app.read_dataset(scope, &dataset_id, page).await?)
            }
            CachedCommand::ReadDocument {
                dataset_id,
                offset,
                length,
            } => self.value(
                app.read_document(scope, &dataset_id, offset, length)
                    .await?,
            ),
            CachedCommand::SelectCandidate {
                dataset_id,
                observation_id,
            } => self.value(
                app.select_candidate(scope, &dataset_id, observation_id)
                    .await?,
            ),
            CachedCommand::OpenView { dataset_id, kind } => self.value(
                app.open_view(scope, OpenViewRequest { dataset_id, kind })
                    .await?,
            ),
            CachedCommand::ReadView { view_id } => {
                self.value(app.read_view(scope, &view_id).await?)
            }
            CachedCommand::JobStatus { job_id } => self.value(app.status(scope, &job_id)?),
            CachedCommand::CancelJob { job_id } => {
                app.cancel(scope, &job_id)?;
                self.value(app.status(scope, &job_id)?)
            }
        }
    }
}
impl ResearchExecutor {
    /// Distinguish host output-budget failure from a returned provider/job failure receipt.
    pub(crate) async fn execute_recorded(&self, call: ToolCall) -> Result<ToolResult> {
        let result = self.dispatch(&call).await;
        let (success, content) = match result {
            Ok((success, value)) => (success, serde_json::to_string(&value)),
            Err(error) if error.kind == ErrorKind::ResourceLimit => return Err(error),
            Err(error) => (false, serde_json::to_string(&error)),
        };
        let result = ToolResult {
            success,
            content: content.map_err(|_| invalid("result encoding failed"))?,
        };
        check_serialized_size(&result, self.output_bytes).map_err(|_| {
            AppError::new(
                ErrorKind::ResourceLimit,
                "tool result exceeds serialized output budget; request a smaller page",
                false,
            )
        })?;
        Ok(result)
    }
}
#[async_trait::async_trait]
impl ToolExecutor for ResearchExecutor {
    async fn execute(&self, call: ToolCall) -> ToolResult {
        self.execute_recorded(call)
            .await
            .unwrap_or_else(|error| ToolResult {
                success: false,
                content: serde_json::to_string(&error).expect("bounded error serializes"),
            })
    }
}

struct CancelOnDrop {
    application: Application,
    scope: Scope,
    id: Option<String>,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(id) = &self.id {
            let _ = self.application.cancel(&self.scope, id);
        }
    }
}
#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum CachedCommand {
    ReadFetch {
        fetch_id: String,
    },
    CreateDataset {
        fetch_id: String,
        projection: DatasetProjection,
    },
    DatasetHeader {
        dataset_id: String,
    },
    ReadDataset {
        dataset_id: String,
        page: PageRequest,
    },
    ReadDocument {
        dataset_id: String,
        offset: usize,
        length: usize,
    },
    SelectCandidate {
        dataset_id: String,
        observation_id: i64,
    },
    OpenView {
        dataset_id: String,
        kind: ViewKind,
    },
    ReadView {
        view_id: String,
    },
    JobStatus {
        job_id: String,
    },
    CancelJob {
        job_id: String,
    },
}
const LOCAL_NAMES: &[&str] = &[
    "read_fetch",
    "create_dataset",
    "dataset_header",
    "read_dataset",
    "read_document",
    "select_candidate",
    "open_view",
    "read_view",
    "job_status",
    "cancel_job",
];
fn invalid(message: &'static str) -> AppError {
    AppError::new(ErrorKind::InvalidInput, message, false)
}
fn object(properties: Vec<(&str, Value)>) -> Value {
    let required: Vec<_> = properties.iter().map(|(k, _)| *k).collect();
    json!({"type":"object","additionalProperties":false,"required":required,"properties":properties.iter().map(|(k,v)| (k.to_string(),v.clone())).collect::<serde_json::Map<_,_>>()})
}
fn id() -> Value {
    json!({"type":"string","minLength":1,"maxLength":Scope::MAX_ID_BYTES})
}
fn integer(max: usize) -> Value {
    json!({"type":"integer","minimum":0,"maximum":max})
}
fn projection_schema() -> Value {
    let kind = |k| json!({"type":"string","const":k});
    let run = || json!({"type":"integer","minimum":1,"maximum":i64::MAX});
    let date = || json!({"type":"string","format":"date"});
    let period = json!({"oneOf":[object(vec![("kind",kind("instant")),("date",date())]),object(vec![("kind",kind("duration")),("start",date()),("end",date())])]});
    let periods = json!({"oneOf":[object(vec![("kind",kind("latest_instant"))]),object(vec![("kind",kind("instants"))]),object(vec![("kind",kind("durations"))]),object(vec![("kind",kind("exact")),("period",period)])]});
    let metric = object(vec![
        ("scope", crate::agent_contract::financial_query_schema()),
        ("namespace", id()),
        ("concept", id()),
        ("unit", id()),
        ("periods", periods),
    ]);
    json!({"oneOf":[
        object(vec![("kind",kind("prices")),("run_id",run()),("query",crate::agent_contract::price_query_schema()),("series",json!({"type":"string","enum":["close","adjusted_close"]}))]),
        object(vec![("kind",kind("facts")),("run_id",run()),("query",metric)]),
        object(vec![("kind",kind("all_facts")),("run_id",run())]),
        object(vec![("kind",kind("filings")),("run_id",run())]),
        object(vec![("kind",kind("resolution")),("run_id",run())]),
        object(vec![("kind",kind("document"))])
    ]})
}
fn cached_tool_specs(limits: &Limits) -> Vec<ToolSpec> {
    let fields = vec![
        (
            "read_fetch",
            "Read an owned durable fetch receipt, including exact run IDs, offline.",
            vec![("fetch_id", id())],
        ),
        (
            "create_dataset",
            "Freeze owned fetch evidence using an explicit supported projection.",
            vec![("fetch_id", id()), ("projection", projection_schema())],
        ),
        (
            "dataset_header",
            "Inspect a frozen dataset and its provenance offline.",
            vec![("dataset_id", id())],
        ),
        (
            "read_dataset",
            "Read a bounded page of a frozen dataset offline.",
            vec![
                ("dataset_id", id()),
                (
                    "page",
                    object(vec![
                        ("offset", integer(usize::MAX)),
                        (
                            "limit",
                            json!({"type":"integer","minimum":1,"maximum":limits.max_read_page_items}),
                        ),
                    ]),
                ),
            ],
        ),
        (
            "read_document",
            "Read bounded original document bytes; no passage extraction is implied.",
            vec![
                ("dataset_id", id()),
                ("offset", integer(usize::MAX)),
                (
                    "length",
                    json!({"type":"integer","minimum":1,"maximum":limits.max_read_page_bytes}),
                ),
            ],
        ),
        (
            "select_candidate",
            "Explicitly select an observed company candidate from an owned resolution dataset.",
            vec![
                ("dataset_id", id()),
                (
                    "observation_id",
                    json!({"type":"integer","minimum":1,"maximum":i64::MAX}),
                ),
            ],
        ),
        (
            "open_view",
            "Accept a view request for an owned dataset; acceptance does not imply rendering.",
            vec![
                ("dataset_id", id()),
                (
                    "kind",
                    json!({"type":"string","enum":["price_chart","data_table","document"]}),
                ),
            ],
        ),
        (
            "read_view",
            "Inspect an accepted view receipt and separately reported presentation state.",
            vec![("view_id", id())],
        ),
        (
            "job_status",
            "Inspect authoritative state of a retained job owned by this workspace.",
            vec![("job_id", id())],
        ),
        (
            "cancel_job",
            "Signal cancellation of an owned job; host supervision finishes cleanup.",
            vec![("job_id", id())],
        ),
    ];
    fields
        .into_iter()
        .map(|(name, description, fields)| ToolSpec {
            name: format!("lugus_{name}"),
            description: description.into(),
            input_schema: object(fields),
        })
        .collect()
}
