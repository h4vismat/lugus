#[allow(dead_code)]
mod conversations_support;
use conversations_support::store;
use lugus_app::{comparison::*, conversations::ConversationLimits, *};
use lugus_financial::comparison::RevenueBasis;
fn scope() -> Scope {
    Scope {
        workspace_id: "w".into(),
        request_id: "r".into(),
        run_id: None,
    }
}
fn request() -> ComparisonRequest {
    serde_json::from_value(serde_json::json!({"request_id":"r","subjects":[{"text":"Apple","exchange":null},{"text":"Microsoft","exchange":null}],"period_end":"2024-12-31","years":3,"revenue_basis":"contract_revenue_excluding_tax"})).unwrap()
}
fn providers() -> CapturedProviders {
    let p = ProviderIdentity {
        instance_id: "sec".into(),
        plugin_id: "sec-edgar".into(),
        plugin_version: "0.3.0".into(),
    };
    CapturedProviders {
        facts: p.clone(),
        resolution: p,
    }
}
#[test]
fn request_replay_scope_and_restart_recovery() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("app");
    let fin = d.path().join("fin");
    let mut s = store(&db, &fin, ConversationLimits::default());
    let sc = scope();
    let lease = s.comparison_store().unwrap().acquire(&sc).unwrap();
    let job = s
        .comparison_store_mut()
        .unwrap()
        .begin(&sc, &request(), &lease, &providers())
        .unwrap();
    assert_eq!(job.state, ComparisonState::Running);
    assert_eq!(
        s.comparison_store()
            .unwrap()
            .lookup_request(&sc, &request())
            .unwrap()
            .unwrap()
            .id,
        job.id
    );
    let mut changed = request();
    changed.revenue_basis = RevenueBasis::Revenues;
    assert_eq!(
        s.comparison_store()
            .unwrap()
            .lookup_request(&sc, &changed)
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    let mut other = sc.clone();
    other.workspace_id = "other".into();
    assert!(s.comparison_store().unwrap().job(&other, &job.id).is_err());
    let second = store(&db, &fin, ConversationLimits::default());
    assert!(second.comparison_store().unwrap().acquire(&sc).is_err());
    drop(second);
    drop(lease);
    drop(s);
    let mut s = store(&db, &fin, ConversationLimits::default());
    let lease = s.comparison_store().unwrap().acquire(&sc).unwrap();
    let recovered = s
        .comparison_store_mut()
        .unwrap()
        .recover_interrupted(&sc, &lease)
        .unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].state, ComparisonState::Interrupted);
    let replay = s
        .comparison_store()
        .unwrap()
        .lookup_request(&sc, &request())
        .unwrap()
        .unwrap();
    assert_eq!(replay.id, job.id);
    assert_eq!(replay.state, ComparisonState::Interrupted);
}
#[test]
fn rejects_foreign_lease_and_terminal_rewrites() {
    let d = tempfile::tempdir().unwrap();
    let mut s = store(
        &d.path().join("a"),
        &d.path().join("f"),
        ConversationLimits::default(),
    );
    let sc = scope();
    let mut other = sc.clone();
    other.workspace_id = "elsewhere".into();
    let lease = s.comparison_store().unwrap().acquire(&other).unwrap();
    assert!(
        s.comparison_store_mut()
            .unwrap()
            .begin(&sc, &request(), &lease, &providers())
            .is_err()
    );
    drop(lease);
    let lease = s.comparison_store().unwrap().acquire(&sc).unwrap();
    let job = s
        .comparison_store_mut()
        .unwrap()
        .begin(&sc, &request(), &lease, &providers())
        .unwrap();
    s.comparison_store_mut()
        .unwrap()
        .finish(&sc, &job.id, ComparisonState::Cancelled, None)
        .unwrap();
    let done = s
        .comparison_store_mut()
        .unwrap()
        .finish(&sc, &job.id, ComparisonState::Failed, None)
        .unwrap();
    assert_eq!(done.state, ComparisonState::Cancelled);
}
