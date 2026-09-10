//! Deterministic AgentRuntime adapter for CLI acceptance, with no model or network dependency.
pub mod conversations;
pub mod passages;
use lugus_agent::reviews::ThesisRevision;
use lugus_agent::{runtime::*, tools::*};
use lugus_app::*;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

pub struct FixtureRuntime {
    pub thesis: ThesisRevision,
    pub command: FixtureCommand,
    pub receipts: Value,
}
#[derive(Clone, serde::Deserialize)]
#[serde(untagged)]
pub enum FixtureCommand {
    Fetch(FetchCommand),
    Binding(BindingWorkflow),
}
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingWorkflow {
    company_instance_id: String,
    market_instance_id: String,
    input: String,
    native_namespace: String,
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
    page_size: usize,
}
impl FixtureCommand {
    pub fn tool_count(&self) -> usize {
        match self {
            Self::Fetch(_) => 5,
            Self::Binding(_) => 12,
        }
    }
}
type AgentResult<T> = lugus_agent::error::Result<T>;
fn failure(message: &str) -> lugus_agent::error::Error {
    lugus_agent::error::Error::Tool(message.into())
}
pub(super) async fn invoke(
    request: &RunRequest,
    tools: &dyn ToolExecutor,
    events: &mpsc::Sender<RuntimeEvent>,
    number: usize,
    name: &str,
    arguments: Value,
) -> AgentResult<Value> {
    if number > request.limits.max_tool_calls {
        return Err(failure("fixture tool count exceeded"));
    }
    if !request.tools.iter().any(|s| s.name == name) {
        return Err(failure("fixture tool was not offered"));
    }
    let call_id = format!("fixture-{number}");
    let _ = events.try_send(RuntimeEvent::ToolStarted {
        call_id: call_id.clone(),
        name: name.into(),
    });
    let result = tools
        .execute(ToolCall {
            run_id: request.run_id.clone(),
            call_id: call_id.clone(),
            name: name.into(),
            arguments,
        })
        .await;
    agent_contract::check_serialized_size(&result, request.limits.max_tool_result_bytes)
        .map_err(|_| failure("fixture result exceeded runtime limit"))?;
    let _ = events.try_send(RuntimeEvent::ToolFinished {
        call_id,
        success: result.success,
    });
    if !result.success {
        return Err(failure("fixture tool returned a failure"));
    }
    serde_json::from_str(&result.content).map_err(|_| failure("fixture tool returned invalid JSON"))
}
impl FixtureCommand {
    async fn sequence(
        &self,
        request: &RunRequest,
        tools: &dyn ToolExecutor,
        events: &mpsc::Sender<RuntimeEvent>,
    ) -> AgentResult<Value> {
        let FixtureCommand::Fetch(command) = self else {
            return self.binding_sequence(request, tools, events).await;
        };
        let mut arguments =
            serde_json::to_value(command).map_err(|_| failure("invalid fixture command"))?;
        let operation = arguments
            .as_object_mut()
            .and_then(|v| v.remove("operation"))
            .ok_or_else(|| failure("missing fixture operation"))?;
        let name = match operation.as_str() {
            Some("filings") => "lugus_fetch_filings",
            Some("prices") => "lugus_fetch_prices",
            Some("document") => "lugus_fetch_document",
            Some("resolve") => "lugus_resolve_company",
            Some("lookup") => "lugus_lookup_company",
            _ => {
                return Err(failure(
                    "fixture supports filings, prices, documents and company resolution",
                ));
            }
        };
        let status = invoke(request, tools, events, 1, name, arguments).await?;
        let fetch = invoke(
            request,
            tools,
            events,
            2,
            "lugus_read_fetch",
            json!({"fetch_id":status["fetch_id"]}),
        )
        .await?;
        let reference: FetchReference =
            serde_json::from_value(fetch.clone()).map_err(|_| failure("invalid fetch receipt"))?;
        let run = || {
            reference
                .runs
                .first()
                .map(|r| r.id)
                .ok_or_else(|| failure("fetch has no run"))
        };
        let projection = match &reference.command {
            FetchCommand::Filings { .. } => DatasetProjection::Filings { run_id: run()? },
            FetchCommand::Prices { query, .. } => DatasetProjection::Prices {
                run_id: run()?,
                query: query.clone(),
                series: lugus_financial::selection::PriceSeries::Close,
            },
            FetchCommand::Document { .. } => DatasetProjection::Document,
            FetchCommand::Resolve { .. } | FetchCommand::Lookup { .. } => {
                DatasetProjection::Resolution { run_id: run()? }
            }
            _ => return Err(failure("unsupported fixture projection")),
        };
        let dataset = invoke(
            request,
            tools,
            events,
            3,
            "lugus_create_dataset",
            json!({"fetch_id":reference.id,"projection":projection}),
        )
        .await?;
        let document = matches!(reference.command, FetchCommand::Document { .. });
        let page = if document {
            invoke(
                request,
                tools,
                events,
                4,
                "lugus_read_document",
                json!({"dataset_id":dataset["id"],"offset":0,"length":256}),
            )
            .await?
        } else {
            invoke(
                request,
                tools,
                events,
                4,
                "lugus_read_dataset",
                json!({"dataset_id":dataset["id"],"page":{"offset":0,"limit":1}}),
            )
            .await?
        };
        let kind = if document {
            "document"
        } else if matches!(reference.command, FetchCommand::Prices { .. }) {
            "price_chart"
        } else {
            "data_table"
        };
        let view = invoke(
            request,
            tools,
            events,
            5,
            "lugus_open_view",
            json!({"dataset_id":dataset["id"],"kind":kind}),
        )
        .await?;
        Ok(json!({"job":status,"fetch":fetch,"dataset":dataset,"page":page,"view":view}))
    }
}
#[async_trait::async_trait]
impl AgentRuntime for FixtureRuntime {
    async fn run(
        &mut self,
        request: RunRequest,
        tools: &dyn ToolExecutor,
        events: mpsc::Sender<RuntimeEvent>,
        mut cancel: watch::Receiver<bool>,
    ) -> AgentResult<RunReport> {
        validate_request(&request)?;
        if request.subject
            != (lugus_agent::RunSubject::Thesis {
                id: self.thesis.thesis_id.clone(),
            })
            || request.context != self.thesis.text
            || request.limits.max_tool_calls < self.command.tool_count()
        {
            return Err(failure(
                "fixture requires its real stored thesis and the complete tool budget",
            ));
        }
        let _ = events.try_send(RuntimeEvent::Started {
            run_id: request.run_id.clone(),
        });
        let outcome = tokio::select! {
            biased;
            _ = async { loop { if *cancel.borrow_and_update() { break; }
                if cancel.changed().await.is_err() { std::future::pending::<()>().await; } } } => RunOutcome::Cancelled,
            result = tokio::time::timeout(request.limits.timeout, self.command.sequence(&request, tools, &events)) => { self.receipts = result.map_err(|_| lugus_agent::error::Error::Timeout)??; RunOutcome::Completed }
        };
        Ok(RunReport { run_id: request.run_id, outcome, final_text: "Research evidence stored and view request accepted; presentation has not been reported.".into() })
    }
    async fn close(&mut self) -> AgentResult<()> {
        Ok(())
    }
}

impl FixtureCommand {
    async fn binding_sequence(
        &self,
        request: &RunRequest,
        tools: &dyn ToolExecutor,
        events: &mpsc::Sender<RuntimeEvent>,
    ) -> AgentResult<Value> {
        let FixtureCommand::Binding(config) = self else {
            unreachable!()
        };
        let resolution_job = invoke(
            request,
            tools,
            events,
            1,
            "lugus_resolve_company",
            json!({"instance_id":config.company_instance_id,"input":config.input}),
        )
        .await?;
        let resolution_fetch = invoke(
            request,
            tools,
            events,
            2,
            "lugus_read_fetch",
            json!({"fetch_id":resolution_job["fetch_id"]}),
        )
        .await?;
        // Resolve may perform an identifier search followed by a name fallback. The
        // terminal run, not the empty initial search, supplies the selected evidence.
        let run = resolution_fetch["runs"]
            .as_array()
            .and_then(|r| r.last())
            .ok_or_else(|| failure("resolution run missing"))?;
        let company_dataset = invoke(request,tools,events,3,"lugus_create_dataset",json!({"fetch_id":resolution_fetch["id"],"projection":{"kind":"resolution","run_id":run["id"]}})).await?;
        let companies = invoke(
            request,
            tools,
            events,
            4,
            "lugus_read_dataset",
            json!({"dataset_id":company_dataset["id"],"page":{"offset":0,"limit":2}}),
        )
        .await?;
        let rows = companies["rows"]
            .as_array()
            .ok_or_else(|| failure("invalid candidate page"))?;
        if rows.len() != 1
            || company_dataset["row_count"] != 1
            || !companies["next_offset"].is_null()
        {
            return Err(failure("explicit company selection required"));
        }
        let company = &rows[0]["entry"];
        let listings = company["candidate"]["listings"]
            .as_array()
            .ok_or_else(|| failure("listing evidence missing"))?;
        if listings.len() != 1 {
            return Err(failure("explicit listing selection required"));
        }
        let listing = &listings[0];
        let lookup = invoke(request,tools,events,5,"lugus_lookup_instrument",json!({"instance_id":config.market_instance_id,"query":{"instrument":{"namespace":config.native_namespace,"value":listing["ticker"]["value"]}}})).await?;
        let instrument_fetch = invoke(
            request,
            tools,
            events,
            6,
            "lugus_read_fetch",
            json!({"fetch_id":lookup["fetch_id"]}),
        )
        .await?;
        let binding = invoke(request,tools,events,7,"lugus_create_binding",json!({"request":{"company_dataset_id":company_dataset["id"],"company_observation_id":company["observation_id"],"listing":listing,"instrument_fetch_id":instrument_fetch["id"],"supersedes":null}})).await?;
        let job = invoke(request,tools,events,8,"lugus_fetch_bound_prices",json!({"binding_id":binding["id"],"start":config.start,"end":config.end,"page_size":config.page_size})).await?;
        let fetch = invoke(
            request,
            tools,
            events,
            9,
            "lugus_read_fetch",
            json!({"fetch_id":job["fetch_id"]}),
        )
        .await?;
        let dataset = invoke(request,tools,events,10,"lugus_create_dataset",json!({"fetch_id":fetch["id"],"projection":{"kind":"prices","run_id":fetch["runs"][0]["id"],"query":fetch["command"]["query"],"series":"close"}})).await?;
        let page = invoke(
            request,
            tools,
            events,
            11,
            "lugus_read_dataset",
            json!({"dataset_id":dataset["id"],"page":{"offset":0,"limit":1}}),
        )
        .await?;
        let view = invoke(
            request,
            tools,
            events,
            12,
            "lugus_open_view",
            json!({"dataset_id":dataset["id"],"kind":"price_chart"}),
        )
        .await?;
        Ok(
            json!({"company_dataset":company_dataset,"companies":companies,"instrument_fetch":instrument_fetch,"binding":binding,"job":job,"fetch":fetch,"dataset":dataset,"page":page,"view":view}),
        )
    }
}
