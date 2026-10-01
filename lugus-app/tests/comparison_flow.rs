mod comparison_support;
mod support;
use comparison_support::*;
use lugus_app::{comparison::*, *};
use support::Harness;
#[tokio::test]
async fn comparison_without_agent_publishes_sources_and_reopens_offline() {
    let h = Harness::with_plugin(
        &[("sec", "apple_comparison")],
        HostBounds::default(),
        Limits::default(),
        "sec-edgar",
        "0.3.0",
    )
    .await;
    let sc = h.scope("first");
    let job = h.app.start_comparison(&sc, request("first")).await.unwrap();
    let done = terminal(&h.app, &sc, &job.id).await;
    assert_eq!(done.state, ComparisonState::Complete, "{:?}", done.error);
    let id = done.comparison_id.unwrap();
    let header = h.app.read_comparison(&sc, &id).await.unwrap();
    let rows = h
        .app
        .comparison_rows(
            &sc,
            &id,
            PageRequest {
                offset: 0,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert_eq!(rows.items.len(), 6);
    assert_eq!(
        rows.items[2]
            .annual
            .revenue_growth
            .result
            .as_ref()
            .unwrap()
            .display,
        "20.00"
    );
    let sources = h
        .app
        .comparison_sources(
            &sc,
            &id,
            PageRequest {
                offset: 0,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert!(!sources.items.is_empty());
    for source in &sources.items {
        let page = h
            .app
            .read_dataset(
                &sc,
                &source.input.dataset_id,
                PageRequest {
                    offset: source.input.ordinal,
                    limit: 1,
                },
            )
            .await
            .unwrap();
        match &page.rows[0] {
            DatasetRow::ReportedFact { evidence } => {
                assert_eq!(evidence.value.value, source.fact.value)
            }
            _ => panic!("wrong source"),
        };
    }
    let package = h
        .app
        .read_research_package(&sc, &header.package_id)
        .await
        .unwrap();
    h.app.shutdown().await.unwrap();
    let app = offline(&h.financial, &h.application).await;
    assert_eq!(
        serde_json::to_value(
            app.read_research_package(&sc, &header.package_id)
                .await
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(package).unwrap()
    );
    assert_eq!(
        serde_json::to_value(
            app.comparison_rows(
                &sc,
                &id,
                PageRequest {
                    offset: 0,
                    limit: 100
                }
            )
            .await
            .unwrap()
        )
        .unwrap(),
        serde_json::to_value(rows).unwrap()
    );
    app.shutdown().await.unwrap();
}
#[tokio::test]
async fn refresh_preserves_history_and_partial_sources() {
    let h = Harness::with_plugin(
        &[("sec", "apple_comparison_partial")],
        HostBounds::default(),
        Limits::default(),
        "sec-edgar",
        "0.3.0",
    )
    .await;
    let sc = h.scope("one");
    let first = h.app.start_comparison(&sc, request("one")).await.unwrap();
    let first = terminal(&h.app, &sc, &first.id).await;
    assert_eq!(first.state, ComparisonState::Partial);
    let old = h
        .app
        .read_comparison(&sc, first.comparison_id.as_ref().unwrap())
        .await
        .unwrap();
    let mut next = request("two");
    next.previous_id = Some(old.id.clone());
    let sc2 = h.scope("two");
    let second = h.app.start_comparison(&sc2, next.clone()).await.unwrap();
    let second = terminal(&h.app, &sc2, &second.id).await;
    assert_eq!(second.state, ComparisonState::Partial);
    assert_ne!(first.comparison_id, second.comparison_id);
    assert_eq!(
        h.app
            .read_comparison(&sc, &old.id)
            .await
            .unwrap()
            .package_id,
        old.package_id
    );
    assert_eq!(
        h.app.start_comparison(&sc2, next).await.unwrap().id,
        second.id
    );
    h.app.shutdown().await.unwrap();
}
#[tokio::test]
async fn validates_requests_and_provider_ambiguity() {
    let h = Harness::with_plugin(
        &[("a", "apple_comparison"), ("b", "apple_comparison")],
        HostBounds::default(),
        Limits::default(),
        "sec-edgar",
        "0.3.0",
    )
    .await;
    assert_eq!(
        h.app
            .start_comparison(&h.scope("r"), request("r"))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::AmbiguousProvider
    );
    let mut r = request("bad");
    r.period_end = "2999-01-01".parse().unwrap();
    assert_eq!(
        h.app
            .start_comparison(&h.scope("bad"), r)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    h.app.shutdown().await.unwrap();
}
