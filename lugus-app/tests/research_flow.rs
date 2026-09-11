mod support;
use lugus_agent::{
    AgentRuntime, RunOutcome, RunReport, RunRequest, RuntimeEvent, ToolCall, ToolExecutor,
};
use lugus_app::{conversations::*, research::*, *};
use serde_json::json;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::{mpsc, watch};

#[derive(Clone)]
struct Model {
    requests: Arc<Mutex<Vec<RunRequest>>>,
    closes: Arc<AtomicUsize>,
    intent: Arc<Mutex<ResearchIntent>>,
    hold: Arc<AtomicBool>,
    attack: Arc<AtomicBool>,
}
#[async_trait::async_trait]
impl RuntimeFactory for Model {
    async fn create(&self) -> Result<Box<dyn AgentRuntime>> {
        Ok(Box::new(self.clone()))
    }
}
#[async_trait::async_trait]
impl AgentRuntime for Model {
    async fn run(
        &mut self,
        request: RunRequest,
        tools: &dyn ToolExecutor,
        _: mpsc::Sender<RuntimeEvent>,
        _: watch::Receiver<bool>,
    ) -> lugus_agent::Result<RunReport> {
        self.requests.lock().unwrap().push(request.clone());
        let text = if request.tools.is_empty() {
            assert!(!request.allow_web_search);
            if self.hold.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            serde_json::to_string(&*self.intent.lock().unwrap()).unwrap()
        } else {
            let package: PreparedResearch = serde_json::from_str(&request.context).unwrap();
            assert_eq!(package.run_id, request.run_id);
            assert!(!request.allow_web_search);
            assert!(request.tools.iter().all(|s| !s.name.contains("fetch_bound")
                && !s.name.starts_with("lugus_fetch_")
                && s.name != "get_price_chart"));
            if self.attack.load(Ordering::SeqCst) {
                let forbidden = tools
                    .execute(ToolCall {
                        run_id: request.run_id.clone(),
                        call_id: "forbidden".into(),
                        name: "lugus_fetch_filings".into(),
                        arguments: json!({"instance_id":"one","query":{}}),
                    })
                    .await;
                assert!(!forbidden.success);
            }
            if let Some(dataset) = package.datasets.iter().find(|d| d.purpose == "daily_close") {
                let local = tools.execute(ToolCall { run_id: request.run_id.clone(), call_id: "read".into(), name: "lugus_read_dataset".into(), arguments: json!({"dataset_id":dataset.page.header.id,"page":{"offset":0,"limit":10}}) }).await;
                assert!(local.success, "{}", local.content);
            }
            "Analysis uses prepared evidence.".into()
        };
        Ok(RunReport {
            run_id: request.run_id,
            outcome: RunOutcome::Completed,
            final_text: text,
        })
    }
    async fn close(&mut self) -> lugus_agent::Result<()> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
fn intent(workflow: Workflow, text: &str) -> ResearchIntent {
    ResearchIntent {
        workflow,
        subjects: vec![SubjectMention {
            text: text.into(),
            exchange: None,
        }],
        start: Some("2024-01-01".parse().unwrap()),
        end: Some("2024-12-31".parse().unwrap()),
        clarification: None,
    }
}
async fn setup(
    mode: &str,
    request: ResearchIntent,
) -> (support::Harness, ConversationHost, Arc<Model>, Conversation) {
    let h = support::Harness::with_plugin(
        &[("one", mode)],
        HostBounds::default(),
        Limits::default(),
        "yfinance",
        "0.2.0",
    )
    .await;
    let model = Arc::new(Model {
        requests: Default::default(),
        closes: Default::default(),
        attack: Default::default(),
        hold: Default::default(),
        intent: Arc::new(Mutex::new(request)),
    });
    let host =
        ConversationHost::start(h.app.clone(), model.clone(), ConversationOptions::default())
            .await
            .unwrap();
    let conversation = host.create("create", "Research").await.unwrap();
    (h, host, model, conversation)
}
fn message(c: &Conversation, id: &str) -> SendMessageRequest {
    SendMessageRequest {
        company_hint: None,
        conversation_id: c.id.clone(),
        request_id: id.into(),
        text: "Show AAPL prices in 2024".into(),
        selected: vec![],
    }
}
async fn finish(host: &ConversationHost, c: &Conversation, id: &str) -> RunRecord {
    let admitted = host.send(message(c, id)).await.unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        host.wait(&c.id, &admitted.id),
    )
    .await
    .unwrap()
    .unwrap()
}
#[tokio::test]
async fn default_chat_prepares_prices_before_analysis_and_persists_exact_injection() {
    let (h, host, model, c) = setup("apple", intent(Workflow::Prices, "AAPL")).await;
    let run = finish(&host, &c, "first").await;
    assert_eq!(run.status, RunStatus::Completed, "{:?}", run.error);
    let requests = model.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    let saved = h
        .app
        .conversation_preparation(&c.id, &run.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved, requests[1].context);
    let prepared: PreparedResearch = serde_json::from_str(&saved).unwrap();
    assert_eq!(
        prepared.subjects[0].company.candidate.identifier.value,
        "0000320193"
    );
    assert!(prepared.subjects[0].binding.is_some());
    let prices = prepared
        .datasets
        .iter()
        .find(|d| d.purpose == "daily_close")
        .unwrap();
    assert_eq!(prices.page.header.row_count, 1);
    assert!(!host.workspace(&c.id).await.unwrap().view_ids.is_empty());
    assert_eq!(host.send(message(&c, "first")).await.unwrap().id, run.id);
    assert_eq!(model.requests.lock().unwrap().len(), 2);
    assert_eq!(model.closes.load(Ordering::SeqCst), 2);
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn fuzzy_company_requires_clarification_before_prices_or_analysis() {
    let (h, host, model, c) = setup("apple", intent(Workflow::Prices, "Apple")).await;
    let run = finish(&host, &c, "first").await;
    assert_eq!(run.status, RunStatus::Completed, "{:?}", run.error);
    let saved = h
        .app
        .conversation_preparation(&c.id, &run.id)
        .await
        .unwrap()
        .unwrap();
    let package: PreparedResearch = serde_json::from_str(&saved).unwrap();
    assert!(package.clarification.is_some());
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    assert!(!h.root.path().join("one/prices-started").exists());
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn source_failure_is_injected_without_fabricating_a_successful_price_dataset() {
    let (h, host, model, c) = setup("apple_price_error", intent(Workflow::Prices, "AAPL")).await;
    let run = finish(&host, &c, "first").await;
    assert_eq!(run.status, RunStatus::Completed, "{:?}", run.error);
    let saved = h
        .app
        .conversation_preparation(&c.id, &run.id)
        .await
        .unwrap()
        .unwrap();
    let package: PreparedResearch = serde_json::from_str(&saved).unwrap();
    assert!(
        package
            .issues
            .iter()
            .any(|e| e.kind == ErrorKind::RateLimited)
    );
    assert!(package.fetches.iter().any(|f| f.error.is_some()));
    assert!(
        package
            .datasets
            .iter()
            .filter(|d| d.purpose == "daily_close")
            .all(|d| d.page.header.row_count == 0)
    );
    assert_eq!(model.requests.lock().unwrap().len(), 2);
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn followup_interpretation_receives_verified_identity_and_saved_evidence_reuse() {
    let (_h, host, model, c) = setup("apple", intent(Workflow::Research, "AAPL")).await;
    let first = finish(&host, &c, "first").await;
    assert_eq!(first.status, RunStatus::Completed, "{:?}", first.error);
    let second = finish(&host, &c, "second").await;
    assert_eq!(second.status, RunStatus::Completed, "{:?}", second.error);
    let requests = model.requests.lock().unwrap().clone();
    assert!(requests[2].context.contains("0000320193"));
    let a: PreparedResearch = serde_json::from_str(&requests[1].context).unwrap();
    let b: PreparedResearch = serde_json::from_str(&requests[3].context).unwrap();
    let first_facts = a
        .fetches
        .iter()
        .find(|f| matches!(f.command, FetchCommand::Facts { .. }))
        .unwrap();
    assert!(b.fetches.iter().any(|f| f.id == first_facts.id));
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancellation_during_preparation_never_starts_analysis() {
    let (h, host, model, c) = setup("apple_price_blocked", intent(Workflow::Prices, "AAPL")).await;
    let run = host.send(message(&c, "first")).await.unwrap();
    h.barrier("one", "prices-started").await;
    host.cancel(&c.id, &run.id).await.unwrap();
    let terminal = host.wait(&c.id, &run.id).await.unwrap();
    assert_eq!(terminal.status, RunStatus::Interrupted);
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    assert!(
        h.app
            .conversation_preparation(&c.id, &run.id)
            .await
            .unwrap()
            .is_none()
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn analysis_fetch_attempt_is_rejected_without_provider_effect() {
    let (h, host, model, c) = setup("apple", intent(Workflow::Prices, "AAPL")).await;
    model.attack.store(true, Ordering::SeqCst);
    let run = finish(&host, &c, "first").await;
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error.unwrap().kind, ErrorKind::Unsupported);
    let sql = rusqlite::Connection::open(&h.financial).unwrap();
    let count: i64 = sql
        .query_row("SELECT count(*) FROM runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "analysis must not fetch filings");
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn exact_name_uses_successful_final_resolution_run_after_ticker_miss() {
    let (h, host, _, c) = setup("apple", intent(Workflow::Prices, "Apple Inc.")).await;
    let run = finish(&host, &c, "first").await;
    assert_eq!(run.status, RunStatus::Completed, "{:?}", run.error);
    let package: PreparedResearch = serde_json::from_str(
        &h.app
            .conversation_preparation(&c.id, &run.id)
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(
        package.clarification.is_none(),
        "{:?}",
        package.clarification
    );
    assert_eq!(package.subjects[0].company.candidate.name, "Apple Inc.");
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn comparison_prepares_two_distinct_company_packages_without_mixing_listings() {
    let mut request = intent(Workflow::Compare, "AAPL");
    request.subjects.push(SubjectMention {
        text: "MSFT".into(),
        exchange: Some("NASDAQ".into()),
    });
    let (h, host, _, c) = setup("apple_compare", request).await;
    let run = finish(&host, &c, "first").await;
    assert_eq!(run.status, RunStatus::Completed, "{:?}", run.error);
    let package: PreparedResearch = serde_json::from_str(
        &h.app
            .conversation_preparation(&c.id, &run.id)
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(
        package.clarification.is_none(),
        "{:?}",
        package.clarification
    );
    assert_eq!(package.subjects.len(), 2);
    assert_eq!(
        package.subjects[0].company.candidate.identifier.value,
        "0000320193"
    );
    assert_eq!(
        package.subjects[1].company.candidate.identifier.value,
        "0000789019"
    );
    assert_ne!(
        package.subjects[0]
            .binding
            .as_ref()
            .unwrap()
            .instrument
            .request
            .instrument,
        package.subjects[1]
            .binding
            .as_ref()
            .unwrap()
            .instrument
            .request
            .instrument
    );
    assert_eq!(
        package
            .datasets
            .iter()
            .filter(|d| d.purpose == "daily_close")
            .count(),
        2
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn conversational_interlude_keeps_verified_context_without_network_refresh() {
    let (h, host, model, c) = setup("apple", intent(Workflow::Prices, "AAPL")).await;
    let first = finish(&host, &c, "first").await;
    assert_eq!(first.status, RunStatus::Completed);
    *model.intent.lock().unwrap() = ResearchIntent {
        workflow: Workflow::Conversation,
        subjects: vec![],
        start: None,
        end: None,
        clarification: None,
    };
    let second = finish(&host, &c, "thanks").await;
    assert_eq!(second.status, RunStatus::Completed, "{:?}", second.error);
    *model.intent.lock().unwrap() = intent(Workflow::Prices, "AAPL");
    let third = finish(&host, &c, "followup").await;
    assert_eq!(third.status, RunStatus::Completed, "{:?}", third.error);
    let requests = model.requests.lock().unwrap().clone();
    assert!(requests[4].context.contains("0000320193"));
    let sql = rusqlite::Connection::open(&h.financial).unwrap();
    let count: i64 = sql
        .query_row("SELECT count(*) FROM market_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "fresh compatible saved prices are reused");
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelling_interpretation_explicitly_closes_runtime() {
    let (_h, host, model, c) = setup("apple", intent(Workflow::Prices, "AAPL")).await;
    model.hold.store(true, Ordering::SeqCst);
    let run = host.send(message(&c, "first")).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while model.requests.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    host.cancel(&c.id, &run.id).await.unwrap();
    assert_eq!(
        host.wait(&c.id, &run.id).await.unwrap().status,
        RunStatus::Interrupted
    );
    assert_eq!(model.closes.load(Ordering::SeqCst), 1);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn fundamentals_opens_complete_reported_facts_and_retains_analysis_metrics() {
    let (h, host, model, c) = setup("apple", intent(Workflow::Research, "AAPL")).await;
    let run = finish(&host, &c, "complete-facts").await;
    assert_eq!(run.status, RunStatus::Completed, "{:?}", run.error);
    let requests = model.requests.lock().unwrap().clone();
    let prepared: PreparedResearch = serde_json::from_str(&requests[1].context).unwrap();
    let executor = ResearchExecutor::new(
        h.app.clone(),
        h.app.scope("workspace", "schema", Some("run")).unwrap(),
    )
    .unwrap();
    let schema = executor
        .tool_specs()
        .iter()
        .find(|t| t.name == "lugus_create_dataset")
        .unwrap();
    let all_facts = schema.input_schema["properties"]["projection"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["properties"]["kind"]["const"] == "all_facts")
        .unwrap();
    assert_eq!(all_facts["additionalProperties"], false);
    assert_eq!(all_facts["required"], json!(["kind", "run_id"]));
    let complete = prepared
        .datasets
        .iter()
        .find(|d| d.purpose == "all_reported_facts")
        .expect("complete facts prepared");
    assert_eq!(
        serde_json::to_value(&complete.page.header.projection).unwrap()["kind"],
        "all_facts"
    );
    assert_eq!(
        prepared
            .datasets
            .iter()
            .filter(|d| matches!(d.page.header.projection, DatasetProjection::Facts { .. }))
            .count(),
        9
    );
    let view = prepared
        .views
        .iter()
        .find(|v| v.dataset_id == complete.page.header.id)
        .expect("complete dataset opened");
    assert_eq!(view.kind, ViewKind::DataTable);
    assert!(
        host.workspace(&c.id)
            .await
            .unwrap()
            .view_ids
            .contains(&view.id)
    );
    assert!(prepared.views.iter().all(|v| {
        !prepared
            .datasets
            .iter()
            .any(|d| d.purpose == "Assets" && d.page.header.id == v.dataset_id)
    }));
    assert!(
        h.app
            .conversation_preparation(&c.id, &run.id)
            .await
            .unwrap()
            .is_some()
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn complete_dataset_storage_failure_keeps_narrow_analysis_available() {
    let (h, host, model, c) = setup("apple", intent(Workflow::Research, "AAPL")).await;
    let db = rusqlite::Connection::open(&h.application).unwrap();
    db.execute_batch("CREATE TRIGGER reject_complete_dataset BEFORE INSERT ON app_records WHEN NEW.category='dataset' AND json_extract(NEW.payload, '$.projection.kind')='all_facts' BEGIN SELECT RAISE(ABORT, 'complete dataset unavailable'); END;").unwrap();
    let run = finish(&host, &c, "failed-complete-facts").await;
    assert_eq!(run.status, RunStatus::Completed, "{:?}", run.error);
    let requests = model.requests.lock().unwrap().clone();
    let prepared: PreparedResearch = serde_json::from_str(&requests[1].context).unwrap();
    assert!(
        !prepared
            .datasets
            .iter()
            .any(|d| d.purpose == "all_reported_facts")
    );
    assert_eq!(
        prepared
            .datasets
            .iter()
            .filter(|d| matches!(d.page.header.projection, DatasetProjection::Facts { .. }))
            .count(),
        9
    );
    assert!(prepared.issues.iter().any(|e| e.kind == ErrorKind::Storage));
    host.shutdown().await.unwrap();
}
