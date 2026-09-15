mod support;
use lugus_agent::tools::{ToolCall, ToolExecutor};
use lugus_app::*;
use serde_json::{Value, json};
use support::*;

fn call(args: Value) -> ToolCall {
    ToolCall {
        run_id: "run".into(),
        call_id: "chart".into(),
        name: "get_price_chart".into(),
        arguments: args,
    }
}
fn args() -> Value {
    json!({"symbol":"AAPL","start":"2024-01-01","end":"2024-01-03"})
}
fn executor(h: &Harness) -> ResearchExecutor {
    ResearchExecutor::new(
        h.app.clone(),
        h.app.scope("workspace", "turn", Some("run")).unwrap(),
    )
    .unwrap()
}
async fn harness(modes: &[(&str, &str)]) -> Harness {
    Harness::with_plugin(
        modes,
        HostBounds::default(),
        Limits::default(),
        "yfinance",
        "0.3.0",
    )
    .await
}
#[tokio::test]
async fn simple_chart_call_fetches_native_symbol_freezes_and_opens_owned_chart() {
    let h = harness(&[("renamed-market", "apple")]).await;
    let e = executor(&h);
    let spec = e
        .tool_specs()
        .iter()
        .find(|s| s.name == "get_price_chart")
        .expect("chart offered");
    assert_eq!(
        spec.input_schema["required"],
        json!(["symbol", "start", "end"])
    );
    assert_eq!(
        spec.input_schema["properties"].as_object().unwrap().len(),
        3
    );
    let result = e.execute(call(args())).await;
    assert!(result.success, "{}", result.content);
    let result: Value = serde_json::from_str(&result.content).unwrap();
    let fetch = h
        .app
        .read_fetch(&h.scope("read"), result["fetch_id"].as_str().unwrap())
        .await
        .unwrap();
    let FetchCommand::Prices { instance_id, query } = fetch.command else {
        panic!("prices expected")
    };
    assert_eq!(instance_id, "renamed-market");
    assert_eq!(query.instrument.namespace, "yahoo:symbol");
    assert_eq!(query.instrument.value, "AAPL");
    assert!(query.cursor.is_none());
    let view = h
        .app
        .read_view(&h.scope("view"), result["view_id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(view.kind, ViewKind::PriceChart);
    assert_eq!(view.dataset_id, result["dataset_id"].as_str().unwrap());
    h.app.shutdown().await.unwrap();
    let page = h
        .app
        .read_dataset(
            &h.scope("offline"),
            &view.dataset_id,
            PageRequest {
                offset: 0,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.header.row_count, 1);
    assert!(page.header.binding_id.is_none()); // Symbol alone never proves company identity.
    assert_eq!(serde_json::to_value(&page.rows).unwrap()[0]["value"], "101");
    let other = h.app.scope("other", "read", None).unwrap();
    assert_eq!(
        h.app.read_view(&other, &view.id).await.unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
}
#[tokio::test]
async fn malformed_or_ambiguous_chart_requests_do_not_fetch() {
    let h = harness(&[("market", "apple")]).await;
    let e = executor(&h);
    for value in [
        json!({"symbol":"","start":"2024-01-01","end":"2024-01-03"}),
        json!({"symbol":"AAPL","start":"2024-02-01","end":"2024-01-03"}),
        json!({"symbol":" AAPL","start":"2024-01-01","end":"2024-01-03"}),
        json!({"symbol":"AAPL","start":"2024-01-01","end":"2024-01-03","namespace":"ticker"}),
    ] {
        let r = e.execute(call(value)).await;
        assert!(!r.success);
        assert_eq!(
            serde_json::from_str::<AppError>(&r.content).unwrap().kind,
            ErrorKind::InvalidInput
        );
    }
    assert!(!h.root.path().join("market/prices-started").exists());
    h.app.shutdown().await.unwrap();
    let h = harness(&[("one", "apple"), ("two", "apple")]).await;
    let r = executor(&h).execute(call(args())).await;
    assert!(!r.success);
    assert_eq!(
        serde_json::from_str::<AppError>(&r.content).unwrap().kind,
        ErrorKind::AmbiguousProvider
    );
    assert!(!h.root.path().join("one/prices-started").exists());
    assert!(!h.root.path().join("two/prices-started").exists());
    h.app.shutdown().await.unwrap();
}
#[tokio::test]
async fn failed_fetch_never_claims_to_open_a_chart_and_unadapted_providers_are_not_offered() {
    let h = harness(&[("market", "apple_price_error")]).await;
    let r = executor(&h).execute(call(args())).await;
    assert!(!r.success);
    let value: Value = serde_json::from_str(&r.content).unwrap();
    assert!(value.get("view_id").is_none());
    assert_eq!(value["state"], "failed");
    h.app.shutdown().await.unwrap();
    let h = Harness::new(
        &[("market", "apple")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    assert!(
        !executor(&h)
            .tool_specs()
            .iter()
            .any(|s| s.name == "get_price_chart")
    );
    h.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelling_a_chart_fetch_reaps_provider_and_does_not_open_a_view() {
    let h = harness(&[("market", "apple_price_blocked")]).await;
    let e = executor(&h);
    let mut events = h.app.subscribe();
    let task = tokio::spawn(async move { e.execute(call(args())).await });
    let job = events.recv().await.unwrap().job.receipt;
    h.barrier("market", "prices-started").await;
    task.abort();
    let _ = task.await;
    let status = h.app.wait(&h.scope("wait"), &job.id).await.unwrap();
    assert_eq!(status.state, JobState::Cancelled);
    let connection = rusqlite::Connection::open(&h.application).unwrap();
    let views: i64 = connection
        .query_row(
            "SELECT count(*) FROM app_records WHERE category='view'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(views, 0);
    h.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn deactivation_after_tool_offering_still_prevents_chart_retrieval() {
    let h = harness(&[("market", "apple")]).await;
    let e = executor(&h);
    h.app.deactivate("market").await.unwrap();
    let result = e.execute(call(args())).await;
    assert!(!result.success);
    assert!(!h.root.path().join("market/prices-started").exists());
    h.app.shutdown().await.unwrap();
}
