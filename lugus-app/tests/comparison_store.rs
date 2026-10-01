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
fn limited_store(
    db: &std::path::Path,
    fin: &std::path::Path,
    limits: Limits,
) -> SqliteApplicationStore {
    SqliteApplicationStore::open(
        db,
        Box::new(lugus_financial::storage::SqliteRepository::open(fin).unwrap()),
        limits,
        Box::new(SystemClock),
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap()
}
fn small_output() -> Limits {
    Limits {
        max_output_bytes: 2304,
        max_read_page_bytes: 2304,
        ..Limits::default()
    }
}
#[test]
fn oversized_receipt_is_rejected_before_durable_admission() {
    let d = tempfile::tempdir().unwrap();
    let mut s = limited_store(&d.path().join("a"), &d.path().join("f"), small_output());
    let sc = scope();
    let lease = s.comparison_store().unwrap().acquire(&sc).unwrap();
    let mut r = request();
    r.question = Some("q".repeat(4096));
    assert_eq!(
        s.comparison_store_mut()
            .unwrap()
            .begin(&sc, &r, &lease, &providers())
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    assert!(
        s.comparison_store()
            .unwrap()
            .lookup_request(&sc, &r)
            .unwrap()
            .is_none()
    );
    s.comparison_store_mut()
        .unwrap()
        .begin(&sc, &request(), &lease, &providers())
        .unwrap();
}
#[test]
fn terminal_persistence_and_orphan_recovery_ignore_smaller_response_limits() {
    for recover in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let db = d.path().join("a");
        let fin = d.path().join("f");
        let mut s = limited_store(&db, &fin, Limits::default());
        let sc = scope();
        let lease = s.comparison_store().unwrap().acquire(&sc).unwrap();
        let mut r = request();
        r.question = Some("q".repeat(4096));
        let j = s
            .comparison_store_mut()
            .unwrap()
            .begin(&sc, &r, &lease, &providers())
            .unwrap();
        drop(lease);
        drop(s);
        let mut s = limited_store(&db, &fin, small_output());
        let lease = s.comparison_store().unwrap().acquire(&sc).unwrap();
        if recover {
            assert_eq!(
                s.comparison_store_mut()
                    .unwrap()
                    .recover_interrupted(&sc, &lease)
                    .unwrap()[0]
                    .state,
                ComparisonState::Interrupted
            );
        } else {
            assert_eq!(
                s.comparison_store_mut()
                    .unwrap()
                    .finish(&sc, &j.id, ComparisonState::Cancelled, None)
                    .unwrap()
                    .state,
                ComparisonState::Cancelled
            );
        }
        assert_eq!(
            s.comparison_store()
                .unwrap()
                .job(&sc, &j.id)
                .unwrap_err()
                .kind,
            ErrorKind::ResourceLimit
        );
        let mut next = request();
        next.request_id = "next".into();
        let mut ns = sc;
        ns.request_id = next.request_id.clone();
        s.comparison_store_mut()
            .unwrap()
            .begin(&ns, &next, &lease, &providers())
            .unwrap();
    }
}
#[cfg(unix)]
#[test]
fn lease_child_probe() {
    let Some(db) = std::env::var_os("LUGUS_TEST_COMPARISON_LEASE_DB") else {
        return;
    };
    let fin = std::env::var_os("LUGUS_TEST_COMPARISON_LEASE_FIN").unwrap();
    let s = limited_store(
        std::path::Path::new(&db),
        std::path::Path::new(&fin),
        Limits::default(),
    );
    let acquired = s.comparison_store().unwrap().acquire(&scope());
    if std::env::var("LUGUS_TEST_COMPARISON_LEASE_EXPECT").unwrap() == "blocked" {
        assert_eq!(acquired.unwrap_err().kind, ErrorKind::Conflict);
    } else {
        assert!(acquired.is_ok());
    }
}
#[cfg(unix)]
#[test]
fn separate_process_and_symlink_share_the_same_workspace_lease() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("a");
    let alias = d.path().join("alias");
    let fin = d.path().join("f");
    let s = limited_store(&db, &fin, Limits::default());
    std::os::unix::fs::symlink(&db, &alias).unwrap();
    let lease = s.comparison_store().unwrap().acquire(&scope()).unwrap();
    let child = |expect: &str| {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "lease_child_probe", "--nocapture"])
            .env("LUGUS_TEST_COMPARISON_LEASE_DB", &alias)
            .env("LUGUS_TEST_COMPARISON_LEASE_FIN", &fin)
            .env("LUGUS_TEST_COMPARISON_LEASE_EXPECT", expect)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    child("blocked");
    drop(lease);
    child("available");
}
