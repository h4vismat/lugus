mod comparison_support;
mod support;
use comparison_support::*;
use lugus_app::{comparison::*, *};
use support::Harness;
#[tokio::test]
async fn cancellation_replays_receipt_and_shutdown_cleans_children() {
    let h = Harness::with_plugin(
        &[("sec", "apple_comparison_blocked")],
        HostBounds::default(),
        Limits::default(),
        "sec-edgar",
        "0.3.0",
    )
    .await;
    let sc = h.scope("r");
    let job = h.app.start_comparison(&sc, request("r")).await.unwrap();
    h.barrier("sec", "comparison-facts").await;
    h.app.cancel_comparison(&sc, &job.id).await.unwrap();
    let done = terminal(&h.app, &sc, &job.id).await;
    assert_eq!(done.state, ComparisonState::Cancelled);
    assert!(!done.fetch_ids.is_empty());
    assert_eq!(
        h.app.start_comparison(&sc, request("r")).await.unwrap().id,
        job.id
    );
    h.app.shutdown().await.unwrap();
}
#[tokio::test]
async fn simultaneous_identical_requests_do_not_duplicate_work() {
    let h = Harness::with_plugin(
        &[("sec", "apple_comparison")],
        HostBounds::default(),
        Limits::default(),
        "sec-edgar",
        "0.3.0",
    )
    .await;
    let sc = h.scope("r");
    let (a, b) = tokio::join!(
        h.app.start_comparison(&sc, request("r")),
        h.app.start_comparison(&sc, request("r"))
    );
    let a = a.unwrap();
    assert_eq!(a.id, b.unwrap().id);
    assert_eq!(
        terminal(&h.app, &sc, &a.id).await.state,
        ComparisonState::Complete
    );
    h.app.shutdown().await.unwrap();
}
#[tokio::test]
async fn shutdown_retains_receipts_and_never_restarts_cancelled_work() {
    let h = Harness::with_plugin(
        &[("sec", "apple_comparison_blocked")],
        HostBounds::default(),
        Limits::default(),
        "sec-edgar",
        "0.3.0",
    )
    .await;
    let sc = h.scope("shutdown");
    let job = h
        .app
        .start_comparison(&sc, request("shutdown"))
        .await
        .unwrap();
    h.barrier("sec", "comparison-facts").await;
    h.app.shutdown().await.unwrap();
    assert_eq!(
        h.app.comparison_status(&sc, &job.id).await.unwrap().state,
        ComparisonState::Cancelled
    );
    let app = offline(&h.financial, &h.application).await;
    assert_eq!(
        app.start_comparison(&sc, request("shutdown"))
            .await
            .unwrap()
            .id,
        job.id
    );
    app.shutdown().await.unwrap();
}
#[tokio::test]
async fn low_pages_and_unlimited_policy_both_prepare_complete_evidence() {
    for mut limits in [Limits::default(), Limits::unlimited_research()] {
        limits.max_read_page_items = 1;
        let h = Harness::with_plugin(
            &[("sec", "apple_comparison")],
            HostBounds::default(),
            limits,
            "sec-edgar",
            "0.3.0",
        )
        .await;
        let sc = h.scope("r");
        let j = h.app.start_comparison(&sc, request("r")).await.unwrap();
        let done = terminal(&h.app, &sc, &j.id).await;
        assert_eq!(done.state, ComparisonState::Complete, "{:?}", done.error);
        let id = done.comparison_id.unwrap();
        let page = h
            .app
            .comparison_rows(
                &sc,
                &id,
                PageRequest {
                    offset: 0,
                    limit: 1,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.next_offset, Some(1));
        h.app.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn timeout_cleans_child_and_no_same_company_financial_fetch() {
    let limits = Limits {
        operation_timeout: std::time::Duration::from_millis(400),
        ..Limits::default()
    };
    let h = Harness::with_plugin(
        &[("sec", "apple_comparison_blocked")],
        HostBounds::default(),
        limits,
        "sec-edgar",
        "0.3.0",
    )
    .await;
    let sc = h.scope("timeout");
    let j = h
        .app
        .start_comparison(&sc, request("timeout"))
        .await
        .unwrap();
    let done = terminal(&h.app, &sc, &j.id).await;
    assert_eq!(done.state, ComparisonState::Failed);
    assert_eq!(done.error.unwrap().kind, ErrorKind::Timeout);
    h.app.shutdown().await.unwrap();
    let h = Harness::with_plugin(
        &[("sec", "apple_comparison")],
        HostBounds::default(),
        Limits::default(),
        "sec-edgar",
        "0.3.0",
    )
    .await;
    let sc = h.scope("same");
    let mut r = request("same");
    r.subjects[1] = r.subjects[0].clone();
    let j = h.app.start_comparison(&sc, r).await.unwrap();
    assert_eq!(
        terminal(&h.app, &sc, &j.id).await.error.unwrap().kind,
        ErrorKind::NeedsAttention
    );
    assert!(!h.root.path().join("sec/comparison-facts").exists());
    h.app.shutdown().await.unwrap();
}
