mod support;
use lugus_app::*;
use serde_json::json;
use support::Harness;
fn command() -> FetchCommand {
    serde_json::from_value(json!({"operation":"historical_prices","instance_id":"p","query":{"instrument":{"namespace":"yahoo:symbol","value":"TEST"},"start":"2026-01-02","end":"2026-01-05","anchor":"2026-01-05","cursor":null,"page_size":2}})).unwrap()
}
#[tokio::test]
async fn managed_history_receipt_and_pages_are_scoped() {
    let h = Harness::new(
        &[("p", "history_ok")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let scope = h.scope("history");
    let job = h.app.submit_manual(&scope, command()).unwrap();
    let status = h.app.wait(&scope, &job.id).await.unwrap();
    let id = status.fetch_id.as_ref().unwrap();
    let fetch = h.app.read_fetch(&scope, id).await.unwrap();
    assert_eq!(fetch.runs[0].kind, RunKind::Historical);
    let page = h
        .app
        .history_evidence_page(
            &scope,
            id,
            PageRequest {
                offset: 0,
                limit: 2,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.next_offset, Some(2));
    assert!(page.items[1].day.close.is_none());
    let other = h.app.scope("another-workspace", "history", None).unwrap();
    assert!(
        h.app
            .history_evidence_page(
                &other,
                id,
                PageRequest {
                    offset: 0,
                    limit: 2
                }
            )
            .await
            .is_err()
    );
    h.app.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancelled_history_never_publishes_readable_success() {
    let h = Harness::new(
        &[("p", "history_blocked")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let scope = h.scope("cancel-history");
    let job = h.app.submit_manual(&scope, command()).unwrap();
    h.barrier("p", "first").await;
    h.app.cancel(&scope, &job.id).unwrap();
    let status = h.app.wait(&scope, &job.id).await.unwrap();
    if let Some(id) = status.fetch_id {
        assert!(
            h.app
                .history_evidence_page(
                    &scope,
                    &id,
                    PageRequest {
                        offset: 0,
                        limit: 2
                    }
                )
                .await
                .is_err()
        );
    }
    h.app.shutdown().await.unwrap();
}
#[tokio::test]
async fn history_pages_obey_worker_budget() {
    let limits = Limits {
        max_pages_per_fetch: 1,
        ..Limits::default()
    };
    let h = Harness::new(&[("p", "history_ok")], HostBounds::default(), limits).await;
    let scope = h.scope("bounded-history");
    let job = h.app.submit_manual(&scope, command()).unwrap();
    let status = h.app.wait(&scope, &job.id).await.unwrap();
    let fetch = h
        .app
        .read_fetch(&scope, status.fetch_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(fetch.error.as_ref().unwrap().kind, ErrorKind::ResourceLimit);
    assert_eq!(fetch.runs[0].kind, RunKind::Historical);
    assert!(
        h.app
            .history_evidence_page(
                &scope,
                &fetch.id,
                PageRequest {
                    offset: 0,
                    limit: 2
                }
            )
            .await
            .is_err()
    );
    h.app.shutdown().await.unwrap();
}
#[tokio::test]
async fn changing_snapshot_metadata_is_a_protocol_failure() {
    let h = Harness::new(
        &[("p", "history_changed_manifest")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let scope = h.scope("changed");
    let job = h.app.submit_manual(&scope, command()).unwrap();
    let status = h.app.wait(&scope, &job.id).await.unwrap();
    let fetch = h
        .app
        .read_fetch(&scope, status.fetch_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(fetch.error.unwrap().kind, ErrorKind::Unavailable);
    assert!(
        !h.app
            .providers()
            .unwrap()
            .iter()
            .find(|p| p.identity.instance_id == "p")
            .unwrap()
            .available
    );
    h.app.shutdown().await.unwrap();
}
