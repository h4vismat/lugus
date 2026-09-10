mod support;
use lugus_agent::tools::{ToolCall, ToolExecutor};
use lugus_app::*;
use serde_json::{Value, json};
use support::*;
fn call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        run_id: "run".into(),
        call_id: "call".into(),
        name: name.into(),
        arguments: args,
    }
}
fn fetch_call(id: &str) -> ToolCall {
    let mut args = serde_json::to_value(command(id)).unwrap();
    args.as_object_mut().unwrap().remove("operation");
    call("lugus_fetch_filings", args)
}
fn error_kind(result: lugus_agent::tools::ToolResult) -> ErrorKind {
    assert!(!result.success);
    serde_json::from_str::<AppError>(&result.content)
        .unwrap()
        .kind
}
#[tokio::test]
async fn executor_uses_bound_scope_and_same_durable_manual_operations() {
    let h = Harness::new(&[("one", "ok")], HostBounds::default(), Limits::default()).await;
    let scope = h.app.scope("workspace", "turn", Some("run")).unwrap();
    let executor = ResearchExecutor::new(h.app.clone(), scope).unwrap();
    let result = executor.execute(fetch_call("one")).await;
    assert!(result.success, "{}", result.content);
    let status: JobStatus = serde_json::from_str(&result.content).unwrap();
    assert_eq!(status.receipt.scope.run_id.as_deref(), Some("run"));
    let fetch = h
        .app
        .read_fetch(&h.scope("manual"), status.fetch_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(fetch.scope, status.receipt.scope);
    let result = executor
        .execute(call(
            "lugus_create_dataset",
            json!({"fetch_id":fetch.id,"projection":projection(&fetch)}),
        ))
        .await;
    assert!(result.success, "{}", result.content);
    let dataset: DatasetHeader = serde_json::from_str(&result.content).unwrap();
    h.app.shutdown().await.unwrap();
    let result = executor
        .execute(call(
            "lugus_read_dataset",
            json!({"dataset_id":dataset.id,"page":{"offset":0,"limit":1}}),
        ))
        .await;
    assert!(result.success, "{}", result.content);
    let page: DatasetPage = serde_json::from_str(&result.content).unwrap();
    assert_eq!(page.rows.len(), 1);
}
#[tokio::test]
async fn strict_tools_reject_forgery_unknown_fields_and_unoffered_instances() {
    let h = Harness::new(
        &[("one", "ok"), ("bad", "startup_failure")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let executor = ResearchExecutor::new(
        h.app.clone(),
        h.app.scope("workspace", "turn", Some("run")).unwrap(),
    )
    .unwrap();
    let mut forged = fetch_call("one");
    forged.run_id = "forged".into();
    assert_eq!(
        error_kind(executor.execute(forged).await),
        ErrorKind::ScopeMismatch
    );
    let mut injected = fetch_call("one");
    injected.arguments["workspace_id"] = json!("other");
    assert_eq!(
        error_kind(executor.execute(injected).await),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        error_kind(executor.execute(fetch_call("bad")).await),
        ErrorKind::Unsupported
    );
    assert_eq!(
        error_kind(
            executor
                .execute(call(
                    "lugus_read_fetch",
                    json!({"fetch_id":"fake","scope":{}})
                ))
                .await
        ),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        error_kind(
            executor
                .execute(call("lugus_report_presentation", json!({})))
                .await
        ),
        ErrorKind::Unsupported
    );
    h.app.activate("bad").await.unwrap_err();
    h.app.shutdown().await.unwrap();
}
#[tokio::test]
async fn cached_tools_always_offered_and_serialized_results_are_bounded_valid_json() {
    let limits = Limits {
        max_input_bytes: 256,
        max_output_bytes: Limits::MIN_OUTPUT_BYTES,
        max_read_page_bytes: Limits::MIN_OUTPUT_BYTES,
        ..Limits::default()
    };
    let h = Harness::new(&[], HostBounds::default(), limits.clone()).await;
    let executor = ResearchExecutor::new(
        h.app.clone(),
        h.app.scope("workspace", "turn", Some("run")).unwrap(),
    )
    .unwrap();
    let names: Vec<_> = executor
        .tool_specs()
        .iter()
        .map(|s| s.name.clone())
        .collect();
    for name in [
        "lugus_read_fetch",
        "lugus_create_dataset",
        "lugus_dataset_header",
        "lugus_read_dataset",
        "lugus_read_document",
        "lugus_select_candidate",
        "lugus_open_view",
        "lugus_read_view",
        "lugus_job_status",
        "lugus_cancel_job",
    ] {
        assert!(names.contains(&name.to_string()), "{name}");
    }
    assert!(!names.contains(&"lugus_fetch_prices".to_string()));
    for call in [
        call("lugus_read_fetch", json!({"fetch_id":"x".repeat(1024)})),
        call("bad", json!({})),
        call(
            "lugus_read_dataset",
            json!({"dataset_id":"fake","page":{"offset":0,"limit":1,"unknown":true}}),
        ),
    ] {
        let result = executor.execute(call).await;
        assert!(!result.success);
        assert!(serde_json::to_vec(&result).unwrap().len() <= limits.max_output_bytes);
        assert!(serde_json::from_str::<Value>(&result.content).is_ok());
    }
    h.app.shutdown().await.unwrap();
}
#[tokio::test]
async fn dropping_tool_wait_cancels_job_and_host_finishes_cleanup() {
    let h = Harness::new(
        &[("one", "blocked")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let executor = ResearchExecutor::new(
        h.app.clone(),
        h.app.scope("workspace", "turn", Some("run")).unwrap(),
    )
    .unwrap();
    let mut events = h.app.subscribe();
    let waiting = tokio::spawn(async move { executor.execute(fetch_call("one")).await });
    let job = events.recv().await.unwrap().job.receipt;
    h.barrier("one", "first").await;
    waiting.abort();
    let _ = waiting.await;
    let status = h.app.wait(&h.scope("manual"), &job.id).await.unwrap();
    assert_eq!(status.state, JobState::Cancelled);
    assert!(status.fetch_id.is_some());
    let pid = std::fs::read_to_string(h.root.path().join("one/pid")).unwrap();
    assert!(
        !std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .output()
            .unwrap()
            .status
            .success()
    );
    h.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn activation_during_a_turn_never_enlarges_its_tools_or_authorization() {
    let h = Harness::new(&[("one", "ok")], HostBounds::default(), Limits::default()).await;
    h.app.deactivate("one").await.unwrap();
    let offering = h.app.offering().unwrap();
    let scope = h.app.scope("workspace", "turn", Some("run")).unwrap();
    let executor = ResearchExecutor::new(h.app.clone(), scope.clone()).unwrap();
    h.app.activate("one").await.unwrap();
    assert_eq!(
        error_kind(executor.execute(fetch_call("one")).await),
        ErrorKind::Unsupported
    );
    assert_eq!(
        h.app
            .submit(&scope, &offering, command("one"))
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    assert!(h.app.submit_manual(&scope, command("one")).is_ok());
    h.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn output_limit_counts_json_escaping_of_entire_tool_result() {
    let limits = Limits {
        max_output_bytes: Limits::MIN_OUTPUT_BYTES,
        max_read_page_bytes: Limits::MIN_OUTPUT_BYTES,
        ..Limits::default()
    };
    let h = Harness::new(&[("one", "ok")], HostBounds::default(), limits.clone()).await;
    let escaped = "\\".repeat(256);
    let scope = h.app.scope(&escaped, "turn", Some(&escaped)).unwrap();
    let executor = ResearchExecutor::new(h.app.clone(), scope.clone()).unwrap();
    let mut events = h.app.subscribe();
    let mut request = fetch_call("one");
    request.run_id = escaped;
    let result = executor.execute(request).await;
    let receipt = events.recv().await.unwrap().job.receipt;
    let terminal = h.app.wait(&scope, &receipt.id).await.unwrap();
    assert_eq!(terminal.state, JobState::Succeeded);
    assert_eq!(error_kind(result.clone()), ErrorKind::ResourceLimit);
    assert!(serde_json::to_vec(&result).unwrap().len() <= limits.max_output_bytes);
    h.app.shutdown().await.unwrap();
}
