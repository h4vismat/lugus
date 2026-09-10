//! Deterministic AgentRuntime adapter for CLI acceptance, with no model or network dependency.
use lugus_agent::reviews::ThesisRevision;
use lugus_agent::{runtime::*, tools::*};
use lugus_app::*;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

pub struct FixtureRuntime {
    pub thesis: ThesisRevision,
    pub command: FetchCommand,
    pub receipts: Value,
}
type AgentResult<T> = lugus_agent::error::Result<T>;
fn failure(message: &str) -> lugus_agent::error::Error {
    lugus_agent::error::Error::Tool(message.into())
}
async fn invoke(
    request: &RunRequest,
    tools: &dyn ToolExecutor,
    events: &mpsc::Sender<RuntimeEvent>,
    number: usize,
    name: &str,
    arguments: Value,
) -> AgentResult<Value> {
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
impl FixtureRuntime {
    async fn sequence(
        &mut self,
        request: &RunRequest,
        tools: &dyn ToolExecutor,
        events: &mpsc::Sender<RuntimeEvent>,
    ) -> AgentResult<()> {
        let mut arguments =
            serde_json::to_value(&self.command).map_err(|_| failure("invalid fixture command"))?;
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
        self.receipts =
            json!({"job":status,"fetch":fetch,"dataset":dataset,"page":page,"view":view});
        Ok(())
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
        if request.thesis_id != self.thesis.thesis_id
            || request.context != self.thesis.text
            || request.limits.max_tool_calls < 5
        {
            return Err(failure(
                "fixture requires its real stored thesis and five tool calls",
            ));
        }
        let _ = events.try_send(RuntimeEvent::Started {
            run_id: request.run_id.clone(),
        });
        let outcome = tokio::select! {
            biased;
            _ = async { loop { if *cancel.borrow_and_update() { break; }
                if cancel.changed().await.is_err() { std::future::pending::<()>().await; } } } => RunOutcome::Cancelled,
            result = tokio::time::timeout(request.limits.timeout, self.sequence(&request, tools, &events)) => { result.map_err(|_| lugus_agent::error::Error::Timeout)??; RunOutcome::Completed }
        };
        Ok(RunReport { run_id: request.run_id, outcome, final_text: "Research evidence stored and view request accepted; presentation has not been reported.".into() })
    }
    async fn close(&mut self) -> AgentResult<()> {
        Ok(())
    }
}
