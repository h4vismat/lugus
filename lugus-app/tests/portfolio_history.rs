mod support;
use lugus_app::portfolio::*;
use lugus_app::*;
use serde_json::json;
use support::Harness;
async fn cash_portfolio(h: &Harness) -> String {
    let p=h.app.execute_portfolio(serde_json::from_value(json!({"request_id":"p","portfolio_id":null,"expected_revision":"0","mutation":{"kind":"create_portfolio","name":"Savings"}})).unwrap()).await.unwrap().portfolio_id;
    h.app.execute_portfolio(serde_json::from_value(json!({"request_id":"a","portfolio_id":p,"expected_revision":"1","mutation":{"kind":"create_account","name":"Cash","start":"2026-01-02","opening":{"kind":"full_history"},"events":[{"id":"deposit","date":"2026-01-02","order":0,"kind":{"kind":"deposit","amount":"100"}}]}})).unwrap()).await.unwrap();
    p
}
async fn completed(h: &Harness, p: &str, id: &str) -> PortfolioHistoryResult {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let r = h
                .app
                .portfolio_history_status(p.into(), id.into())
                .await
                .unwrap();
            if r.status != HistoryStatus::Running {
                return r;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn portfolio_only_history_survives_without_a_provider_and_is_cached() {
    let h = Harness::new(&[], HostBounds::default(), Limits::default()).await;
    let p = cash_portfolio(&h).await;
    let request:PortfolioHistoryRequest=serde_json::from_value(json!({"request_id":"history","portfolio_id":p,"account_id":null,"expected_revision":"2","range":{"start":"2026-01-02","end":"2026-01-05"},"refresh":"missing"})).unwrap();
    let started = h
        .app
        .start_portfolio_history(request.clone())
        .await
        .unwrap();
    let result = completed(&h, &p, &started.id).await;
    assert_eq!(result.status, HistoryStatus::Partial);
    assert_eq!(
        result.summary.portfolio_return_percent.unwrap().to_string(),
        "0"
    );
    assert!(result.summary.benchmark_return_percent.is_none());
    let page = h
        .app
        .portfolio_history_page(
            p.clone(),
            result.id.clone(),
            PageRequest {
                offset: 0,
                limit: 200,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 5);
    assert_eq!(page.items[1].value.as_ref().unwrap().to_string(), "100");
    assert_eq!(
        h.app.start_portfolio_history(request).await.unwrap().id,
        result.id
    );
    assert!(
        h.app
            .portfolio_history_status("wrong".into(), result.id.clone())
            .await
            .is_err()
    );
    h.app.shutdown().await.unwrap();
}
#[tokio::test]
async fn benchmark_history_runs_without_an_agent_runtime() {
    let h = Harness::with_plugin(
        &[("p", "history_ok")],
        HostBounds::default(),
        Limits::default(),
        "yfinance",
        "0.3.0",
    )
    .await;
    let p = cash_portfolio(&h).await;
    let request=serde_json::from_value(json!({"request_id":"history","portfolio_id":p,"account_id":null,"expected_revision":"2","range":{"start":"2026-01-02","end":"2026-01-05"},"refresh":"force"})).unwrap();
    let started = h.app.start_portfolio_history(request).await.unwrap();
    let result = completed(&h, &p, &started.id).await;
    assert_eq!(
        result
            .summary
            .benchmark_return_percent
            .as_ref()
            .map(ToString::to_string),
        Some("0".into()),
        "{:?}",
        result
    );
    h.app.shutdown().await.unwrap();
}
#[tokio::test]
async fn history_cancellation_and_shutdown_release_ownership() {
    let h = Harness::with_plugin(
        &[("p", "history_blocked")],
        HostBounds::default(),
        Limits::default(),
        "yfinance",
        "0.3.0",
    )
    .await;
    let p = cash_portfolio(&h).await;
    let request:PortfolioHistoryRequest=serde_json::from_value(json!({"request_id":"history","portfolio_id":p,"account_id":null,"expected_revision":"2","range":{"start":"2026-01-02","end":"2026-01-05"},"refresh":"force"})).unwrap();
    let started = h
        .app
        .start_portfolio_history(request.clone())
        .await
        .unwrap();
    h.barrier("p", "first").await;
    assert_eq!(
        h.app.start_portfolio_history(request).await.unwrap().id,
        started.id
    );
    assert!(
        h.app
            .cancel_portfolio_history("wrong".into(), started.id.clone())
            .is_err()
    );
    h.app
        .cancel_portfolio_history(p.clone(), started.id.clone())
        .unwrap();
    assert_eq!(
        completed(&h, &p, &started.id).await.status,
        HistoryStatus::Cancelled
    );
    h.app.shutdown().await.unwrap();
}
