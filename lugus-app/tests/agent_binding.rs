mod support;
use lugus_agent::tools::{ToolCall, ToolExecutor};
use lugus_app::*;
use serde_json::{Value, json};
use support::*;

async fn invoke(executor: &ResearchExecutor, number: &str, name: &str, arguments: Value) -> Value {
    let result = executor
        .execute(ToolCall {
            run_id: "run".into(),
            call_id: number.into(),
            name: format!("lugus_{name}"),
            arguments,
        })
        .await;
    assert!(result.success, "{name}: {}", result.content);
    serde_json::from_str(&result.content).unwrap()
}
async fn binding(h: &Harness) -> BindingRecord {
    let e = ResearchExecutor::new(
        h.app.clone(),
        h.app.scope("workspace", "turn", Some("run")).unwrap(),
    )
    .unwrap();
    let job = invoke(
        &e,
        "resolve",
        "resolve_company",
        json!({"instance_id":"one","input":"Apple"}),
    )
    .await;
    let fetch = invoke(
        &e,
        "company-fetch",
        "read_fetch",
        json!({"fetch_id":job["fetch_id"]}),
    )
    .await;
    let dataset = invoke(&e, "company-dataset", "create_dataset", json!({"fetch_id":fetch["id"],"projection":{"kind":"resolution","run_id":fetch["runs"].as_array().unwrap().last().unwrap()["id"]}})).await;
    let page = invoke(
        &e,
        "company-page",
        "read_dataset",
        json!({"dataset_id":dataset["id"],"page":{"offset":0,"limit":10}}),
    )
    .await;
    let row = &page["rows"][0]["entry"];
    let lookup = invoke(&e, "lookup", "lookup_instrument", json!({"instance_id":"one","query":{"instrument":{"namespace":"yahoo:symbol","value":"AAPL"}}})).await;
    let result = invoke(&e, "bind", "create_binding", json!({"request":{"company_dataset_id":dataset["id"],"company_observation_id":row["observation_id"],"listing":row["candidate"]["listings"][0],"instrument_fetch_id":lookup["fetch_id"],"supersedes":null}})).await;
    serde_json::from_value(result).unwrap()
}
fn prices(id: &str) -> BoundPriceRequest {
    BoundPriceRequest {
        binding_id: id.into(),
        start: "2024-01-01".parse().unwrap(),
        end: "2024-01-03".parse().unwrap(),
        page_size: 10,
    }
}
#[tokio::test]
async fn automatic_binding_fetches_exact_instrument_and_retains_original_dataset_after_supersession()
 {
    let h = Harness::new(
        &[("one", "apple")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let b = binding(&h).await;
    assert_eq!(b.policy, "instrument-binding-v1");
    assert!(b.reasons.iter().any(|r| r.contains("source_supported")));
    assert_eq!(b.scope.run_id.as_deref(), Some("run"));
    let scope = h.scope("price");
    let job = h
        .app
        .fetch_bound_prices_manual(&scope, &prices(&b.id))
        .await
        .unwrap();
    let status = h.app.wait(&scope, &job.id).await.unwrap();
    assert_eq!(status.state, JobState::Succeeded);
    let fetch = h
        .app
        .read_fetch(&scope, status.fetch_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(fetch.binding_id.as_deref(), Some(b.id.as_str()));
    let FetchCommand::Prices { query, instance_id } = &fetch.command else {
        panic!("prices")
    };
    assert_eq!(instance_id, "one");
    assert_eq!(query.instrument.value, "AAPL");
    let dataset = h
        .app
        .create_dataset(
            &h.scope("dataset"),
            &fetch.id,
            DatasetProjection::Prices {
                run_id: fetch.runs[0].id,
                query: query.clone(),
                series: lugus_financial::selection::PriceSeries::Close,
            },
        )
        .await
        .unwrap();
    let replacement = h
        .app
        .create_binding(
            &h.scope("replacement"),
            &BindRequest {
                company_dataset_id: b.company_dataset_id.clone(),
                company_observation_id: b.company.observation_id,
                listing: b.listing.clone(),
                instrument_fetch_id: b.instrument_fetch_id.clone(),
                supersedes: Some(b.id.clone()),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        h.app
            .fetch_bound_prices_manual(&h.scope("old"), &prices(&b.id))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    assert_eq!(
        h.app
            .dataset_header(&scope, &dataset.id)
            .await
            .unwrap()
            .binding_id,
        Some(b.id.clone())
    );
    assert_eq!(
        h.app.read_binding(&scope, &b.id).await.unwrap().status,
        BindingStatus::Superseded {
            binding_id: replacement.id
        }
    );
    h.app.shutdown().await.unwrap();
    assert_eq!(
        h.app
            .binding_history(
                &scope,
                &b.id,
                PageRequest {
                    offset: 0,
                    limit: 10
                }
            )
            .await
            .unwrap()
            .events
            .len(),
        2
    );
}

#[tokio::test]
async fn prepared_bound_job_keeps_provenance_when_tool_is_dropped_and_binding_revoked() {
    let h = Harness::new(
        &[("one", "apple_price_blocked")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let b = binding(&h).await;
    let e = ResearchExecutor::new(
        h.app.clone(),
        h.app
            .scope("workspace", "prices-turn", Some("run"))
            .unwrap(),
    )
    .unwrap();
    let mut events = h.app.subscribe();
    let args = serde_json::to_value(prices(&b.id)).unwrap();
    let waiting = tokio::spawn(async move {
        e.execute(ToolCall {
            run_id: "run".into(),
            call_id: "prices".into(),
            name: "lugus_fetch_bound_prices".into(),
            arguments: args,
        })
        .await
    });
    let job = events.recv().await.unwrap().job.receipt;
    h.barrier("one", "prices-started").await;
    h.app
        .revoke_binding(
            &h.scope("revoke"),
            &RevokeBindingRequest {
                binding_id: b.id.clone(),
            },
        )
        .await
        .unwrap();
    waiting.abort();
    let _ = waiting.await;
    let terminal = h.app.wait(&h.scope("wait"), &job.id).await.unwrap();
    assert_eq!(terminal.state, JobState::Cancelled);
    let fetch = h
        .app
        .read_fetch(&h.scope("read"), terminal.fetch_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(fetch.binding_id, Some(b.id.clone()));
    assert_eq!(fetch.error.unwrap().kind, ErrorKind::Cancelled);
    assert_eq!(
        h.app
            .fetch_bound_prices_manual(&h.scope("later"), &prices(&b.id))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    h.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn schema_prices_match_runtime_serialization() {
    let h = Harness::new(&[], HostBounds::default(), Limits::default()).await;
    let e = ResearchExecutor::new(
        h.app.clone(),
        h.app.scope("workspace", "turn", Some("run")).unwrap(),
    )
    .unwrap();
    let spec = e
        .tool_specs()
        .iter()
        .find(|s| s.name == "lugus_create_dataset")
        .unwrap();
    let prices = &spec.input_schema["properties"]["projection"]["oneOf"][0];
    assert!(
        prices["properties"]["series"]["enum"]
            .as_array()
            .unwrap()
            .contains(
                &serde_json::to_value(lugus_financial::selection::PriceSeries::Close).unwrap()
            )
    );
    h.app.shutdown().await.unwrap();
}

fn tool(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        run_id: "run".into(),
        call_id: "test".into(),
        name: format!("lugus_{name}"),
        arguments,
    }
}
async fn rejected(e: &ResearchExecutor, name: &str, args: Value, kind: ErrorKind) {
    let result = e.execute(tool(name, args)).await;
    assert!(!result.success, "{}", result.content);
    assert_eq!(
        serde_json::from_str::<AppError>(&result.content)
            .unwrap()
            .kind,
        kind,
        "{}",
        result.content
    );
}
#[tokio::test]
async fn binding_tools_reject_scope_authority_injection_and_preserve_captured_offerings() {
    let h = Harness::new(
        &[("one", "apple"), ("bad", "startup_failure")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let b = binding(&h).await;
    let e = ResearchExecutor::new(
        h.app.clone(),
        h.app.scope("workspace", "turn", Some("run")).unwrap(),
    )
    .unwrap();
    let request = json!({"company_dataset_id":b.company_dataset_id,"company_observation_id":b.company.observation_id,"listing":b.listing,"instrument_fetch_id":b.instrument_fetch_id,"supersedes":null});
    for field in [
        "scope",
        "actor",
        "verified",
        "policy",
        "metadata",
        "workspace_id",
        "run_id",
    ] {
        let mut forged = request.clone();
        forged[field] = json!(true);
        rejected(
            &e,
            "create_binding",
            json!({"request":forged}),
            ErrorKind::InvalidInput,
        )
        .await;
    }
    for pointer in ["/listing", "/listing/ticker", "/listing/exchange"] {
        let mut forged = request.clone();
        forged.pointer_mut(pointer).unwrap()["verified"] = json!(true);
        rejected(
            &e,
            "create_binding",
            json!({"request":forged}),
            ErrorKind::InvalidInput,
        )
        .await;
    }
    for field in ["instrument", "instance_id", "scope", "binding", "verified"] {
        let mut forged = serde_json::to_value(prices(&b.id)).unwrap();
        forged[field] = json!("replacement");
        rejected(&e, "fetch_bound_prices", forged, ErrorKind::InvalidInput).await;
    }
    rejected(&e,"lookup_instrument",json!({"instance_id":"bad","query":{"instrument":{"namespace":"yahoo:symbol","value":"AAPL"}}}),ErrorKind::Unsupported).await;
    let other = ResearchExecutor::new(
        h.app.clone(),
        h.app.scope("other", "turn", Some("run")).unwrap(),
    )
    .unwrap();
    rejected(
        &other,
        "read_binding",
        json!({"binding_id":b.id}),
        ErrorKind::ScopeMismatch,
    )
    .await;
    rejected(
        &other,
        "create_binding",
        json!({"request":request}),
        ErrorKind::ScopeMismatch,
    )
    .await;
    rejected(
        &other,
        "fetch_bound_prices",
        serde_json::to_value(prices(&b.id)).unwrap(),
        ErrorKind::ScopeMismatch,
    )
    .await;
    let page = invoke(
        &e,
        "list",
        "list_bindings",
        json!({"page":{"offset":0,"limit":10}}),
    )
    .await;
    assert_eq!(page["bindings"].as_array().unwrap().len(), 1);
    rejected(
        &e,
        "list_bindings",
        json!({"page":{"offset":0,"limit":0}}),
        ErrorKind::ResourceLimit,
    )
    .await;
    h.app.deactivate("one").await.unwrap();
    let unavailable = ResearchExecutor::new(
        h.app.clone(),
        h.app
            .scope("workspace", "unavailable", Some("run"))
            .unwrap(),
    )
    .unwrap();
    h.app.activate("one").await.unwrap();
    rejected(&unavailable,"lookup_instrument",json!({"instance_id":"one","query":{"instrument":{"namespace":"yahoo:symbol","value":"AAPL"}}}),ErrorKind::Unsupported).await;
    rejected(
        &unavailable,
        "fetch_bound_prices",
        serde_json::to_value(prices(&b.id)).unwrap(),
        ErrorKind::Unsupported,
    )
    .await;
    rejected(
        &e,
        "fetch_bound_prices",
        serde_json::to_value(prices(&b.id)).unwrap(),
        ErrorKind::StaleReference,
    )
    .await;
    // A new turn admits a same-version process restart with the old immutable binding.
    let fresh = ResearchExecutor::new(
        h.app.clone(),
        h.app.scope("workspace", "fresh", Some("run")).unwrap(),
    )
    .unwrap();
    let result = invoke(
        &fresh,
        "fresh-prices",
        "fetch_bound_prices",
        serde_json::to_value(prices(&b.id)).unwrap(),
    )
    .await;
    assert_eq!(result["state"], "succeeded");
    let revoked = invoke(
        &fresh,
        "revoke",
        "revoke_binding",
        json!({"binding_id":b.id}),
    )
    .await;
    assert_eq!(revoked["status"]["status"], "revoked");
    rejected(
        &fresh,
        "fetch_bound_prices",
        serde_json::to_value(prices(&b.id)).unwrap(),
        ErrorKind::Conflict,
    )
    .await;
    h.app.shutdown().await.unwrap();
    let history = invoke(
        &fresh,
        "history",
        "binding_history",
        json!({"binding_id":b.id,"page":{"offset":0,"limit":10}}),
    )
    .await;
    assert_eq!(history["events"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn bound_fetch_rejects_changed_provider_version_without_dispatch() {
    use lugus_financial::{plugin::Manifest, storage::SqliteRepository};
    use std::sync::Arc;
    let h = Harness::new(
        &[("one", "apple")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let b = binding(&h).await;
    h.app.shutdown().await.unwrap();
    let limits = Limits::default();
    let store = SqliteApplicationStore::open(
        &h.application,
        Box::new(SqliteRepository::open(&h.financial).unwrap()),
        limits.clone(),
        Box::new(SystemClock),
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap();
    let barrier = h.root.path().join("changed");
    let app = Application::start(
        vec![ConfiguredProvider {
            active: true,
            factory: Arc::new(ProcessProviderFactory {
                manifest: Manifest {
                    id: "worker-fixture".into(),
                    version: "2".into(),
                    protocol_version: 1,
                    command: "python3".into(),
                    args: vec![format!(
                        "{}/tests/fixtures/worker.py",
                        env!("CARGO_MANIFEST_DIR")
                    )],
                },
                directory: h.root.path().into(),
                instance_id: "one".into(),
                config: json!({"mode":"apple","version":"2","barrier":barrier}),
            }),
        }],
        Arc::new(SqliteRepositoryFactory::new(&h.financial)),
        Box::new(store),
        limits,
        HostBounds::default(),
        Box::new(RandomIds::new().unwrap()),
    )
    .await
    .unwrap();
    assert!(app.offering().unwrap().supports("one", Operation::Prices));
    assert_eq!(
        app.fetch_bound_prices_manual(&h.scope("changed"), &prices(&b.id))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::StaleReference
    );
    assert!(!barrier.join("prices-started").exists());
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn prepared_fetch_survives_revocation_and_full_admission_does_not_fabricate_receipts() {
    let h = Harness::new(
        &[("one", "apple_price_blocked")],
        HostBounds {
            max_pending_jobs: 1,
            ..HostBounds::default()
        },
        Limits::default(),
    )
    .await;
    let b = binding(&h).await;
    let job = h
        .app
        .fetch_bound_prices_manual(&h.scope("first"), &prices(&b.id))
        .await
        .unwrap();
    h.barrier("one", "prices-started").await;
    assert_eq!(
        h.app
            .fetch_bound_prices_manual(&h.scope("full"), &prices(&b.id))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    h.app
        .revoke_binding(
            &h.scope("revoke"),
            &RevokeBindingRequest {
                binding_id: b.id.clone(),
            },
        )
        .await
        .unwrap();
    std::fs::write(h.root.path().join("one/release"), "").unwrap();
    let terminal = h.app.wait(&h.scope("wait"), &job.id).await.unwrap();
    assert_eq!(terminal.state, JobState::Succeeded);
    let fetch = h
        .app
        .read_fetch(&h.scope("read"), terminal.fetch_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(fetch.binding_id, Some(b.id));
    h.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_bound_prices_keep_binding_and_honest_error() {
    let h = Harness::new(
        &[("one", "apple_price_error")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let b = binding(&h).await;
    let job = h
        .app
        .fetch_bound_prices_manual(&h.scope("fail"), &prices(&b.id))
        .await
        .unwrap();
    let terminal = h.app.wait(&h.scope("wait"), &job.id).await.unwrap();
    assert_eq!(terminal.state, JobState::Failed);
    let fetch = h
        .app
        .read_fetch(&h.scope("read"), terminal.fetch_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(fetch.binding_id, Some(b.id));
    assert_eq!(fetch.error.unwrap().kind, ErrorKind::RateLimited);
    h.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn binding_tool_output_budget_counts_complete_escaped_envelope() {
    use lugus_financial::storage::SqliteRepository;
    use std::sync::Arc;
    let h = Harness::new(
        &[("one", "apple_escaped")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let b = binding(&h).await;
    let view = h.app.read_binding(&h.scope("view"), &b.id).await.unwrap();
    let size = serde_json::to_vec(&view).unwrap().len();
    h.app.shutdown().await.unwrap();
    let limits = Limits {
        max_output_bytes: size,
        max_read_page_bytes: size,
        ..Limits::default()
    };
    let store = SqliteApplicationStore::open(
        &h.application,
        Box::new(SqliteRepository::open(&h.financial).unwrap()),
        Limits::default(),
        Box::new(SystemClock),
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap();
    let app = Application::start(
        vec![],
        Arc::new(SqliteRepositoryFactory::new(&h.financial)),
        Box::new(store),
        limits,
        HostBounds::default(),
        Box::new(RandomIds::new().unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(
        app.read_binding(&h.scope("manual"), &b.id)
            .await
            .unwrap()
            .record
            .id,
        b.id
    );
    let e = ResearchExecutor::new(
        app.clone(),
        app.scope("workspace", "turn", Some("run")).unwrap(),
    )
    .unwrap();
    let result = e
        .execute(tool("read_binding", json!({"binding_id":b.id})))
        .await;
    assert!(!result.success);
    assert_eq!(
        serde_json::from_str::<AppError>(&result.content)
            .unwrap()
            .kind,
        ErrorKind::ResourceLimit
    );
    assert!(serde_json::to_vec(&result).unwrap().len() <= size);
    app.shutdown().await.unwrap();
}
