use lugus_app::*;
use lugus_financial::{
    plugin::Manifest,
    storage::{Repository, RunStatus, SqliteRepository},
};
use serde_json::json;
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

struct Harness {
    _root: tempfile::TempDir,
    barrier: std::path::PathBuf,
    db: std::path::PathBuf,
    catalog: Arc<Mutex<Catalog>>,
    worker: WorkerHandle,
}
fn command(id: &str) -> FetchCommand {
    FetchCommand::Facts {
        instance_id: id.into(),
        query: Query {
            company: lugus_financial::domain::CompanyId {
                namespace: "sec:cik".into(),
                value: "0000320193".into(),
            },
            filed_from: "2024-01-01".parse().unwrap(),
            filed_to: "2024-12-31".parse().unwrap(),
            forms: vec![],
            page_size: 10,
            cursor: None,
        },
    }
}
fn scope() -> Scope {
    Scope {
        workspace_id: "w".into(),
        request_id: "r".into(),
        run_id: None,
    }
}
async fn barrier(path: &Path, name: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.join(name).exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("fixture response barrier");
}
fn reaped(path: &Path) {
    let pid = std::fs::read_to_string(path.join("pid")).unwrap();
    let output = std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .output()
        .unwrap();
    assert!(!output.status.success(), "child is still alive or unreaped");
}
impl Harness {
    async fn start(mode: &str, limits: Limits) -> Self {
        Self::start_shared(
            mode,
            limits.clone(),
            WorkerSlots::new(limits.max_concurrent_jobs).unwrap(),
        )
        .await
    }
    async fn start_shared(mode: &str, limits: Limits, slots: WorkerSlots) -> Self {
        let root = tempfile::tempdir().unwrap();
        let barrier = root.path().join("barrier");
        let db = root.path().join("evidence.sqlite");
        let factory = SqliteRepositoryFactory::new(&db);
        factory.initialize().unwrap();
        let catalog = Arc::new(Mutex::new(Catalog::new(vec![]).unwrap()));
        let provider = ProcessProviderFactory {
            manifest: Manifest {
                id: "worker-fixture".into(),
                version: "1".into(),
                protocol_version: 1,
                command: "python3".into(),
                args: vec![format!(
                    "{}/tests/fixtures/worker.py",
                    env!("CARGO_MANIFEST_DIR")
                )],
            },
            directory: root.path().to_path_buf(),
            instance_id: "fixture".into(),
            config: json!({"mode": mode, "barrier": barrier}),
        };
        // Configured identity is known before startup; only a negotiated process becomes available.
        *catalog.lock().unwrap() = Catalog::new(vec![ProviderEntry {
            identity: ProviderIdentity {
                instance_id: "fixture".into(),
                plugin_id: "worker-fixture".into(),
                plugin_version: "1".into(),
            },
            active: true,
            available: false,
            capabilities: Default::default(),
        }])
        .unwrap();
        let worker = WorkerHandle::start(
            Arc::new(provider),
            Arc::new(factory),
            catalog.clone(),
            limits.clone(),
            slots,
        )
        .await
        .unwrap();
        Self {
            _root: root,
            barrier,
            db,
            catalog,
            worker,
        }
    }
    fn submit(&self, command: FetchCommand) -> Result<JobHandle> {
        let offering = self.catalog.lock().unwrap().snapshot();
        self.worker.submit(offering, scope(), command)
    }
}
#[tokio::test]
async fn source_failures_preserve_worker_and_failed_run_before_successful_retry() {
    for (mode, kind) in [
        ("source_timeout_once", ErrorKind::Timeout),
        ("source_unavailable_once", ErrorKind::Unavailable),
    ] {
        let h = Harness::start(mode, Limits::default()).await;
        let first = h.submit(command("fixture")).unwrap().wait().await.unwrap();
        assert_eq!(first.error.as_ref().unwrap().kind, kind);
        assert!(h.catalog.lock().unwrap().get("fixture").unwrap().available);
        let second = h.submit(command("fixture")).unwrap().wait().await.unwrap();
        assert!(second.error.is_none());
        let FetchCommand::Facts { query, .. } = command("fixture") else {
            unreachable!()
        };
        let repo = SqliteRepository::open(&h.db).unwrap();
        let snapshot = repo.snapshot(&second.provenance.provider, &query).unwrap();
        assert_eq!(snapshot.runs.len(), 2);
        assert!(
            snapshot
                .runs
                .iter()
                .any(|r| r.id == first.provenance.runs[0].id && r.status == RunStatus::Failed)
        );
        assert!(
            snapshot
                .runs
                .iter()
                .any(|r| r.id == second.provenance.runs[0].id && r.status == RunStatus::Complete)
        );
        h.worker.shutdown().await.unwrap();
        reaped(&h.barrier);
    }
}

#[tokio::test]
async fn cancellation_finalizes_started_run_preserves_page_and_reaps() {
    let h = Harness::start("second_blocked", Limits::default()).await;
    let job = h.submit(command("fixture")).unwrap();
    barrier(&h.barrier, "second").await;
    assert_eq!(job.state(), JobState::Running);
    job.cancel();
    let result = job.wait().await.unwrap();
    assert_eq!(result.error.unwrap().kind, ErrorKind::Cancelled);
    assert_eq!(result.provenance.runs.len(), 1);
    let FetchCommand::Facts { query, .. } = command("fixture") else {
        unreachable!()
    };
    let repo = SqliteRepository::open(&h.db).unwrap();
    let snapshot = repo.snapshot(&result.provenance.provider, &query).unwrap();
    assert_eq!(snapshot.facts.len(), 1);
    assert_eq!(snapshot.runs[0].id, result.provenance.runs[0].id);
    assert_eq!(snapshot.runs[0].status, RunStatus::Failed);
    assert!(!h.catalog.lock().unwrap().get("fixture").unwrap().available);
    reaped(&h.barrier);
    h.worker.shutdown().await.unwrap();
}
#[tokio::test]
async fn blocked_instance_does_not_block_other_instance() {
    let blocked = Harness::start("blocked", Limits::default()).await;
    let fast = Harness::start("ok", Limits::default()).await;
    let job = blocked.submit(command("fixture")).unwrap();
    barrier(&blocked.barrier, "first").await;
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        fast.submit(command("fixture")).unwrap().wait(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(result.error.is_none());
    job.cancel();
    job.wait().await.unwrap();
    blocked.worker.shutdown().await.unwrap();
    fast.worker.shutdown().await.unwrap();
}
#[tokio::test]
async fn queue_is_bounded_and_queued_cancel_does_not_start_run() {
    let h = Harness::start(
        "blocked",
        Limits {
            queue_capacity: 1,
            ..Limits::default()
        },
    )
    .await;
    let first = h.submit(command("fixture")).unwrap();
    barrier(&h.barrier, "first").await;
    let queued = h.submit(command("fixture")).unwrap();
    assert!(matches!(
        h.submit(command("fixture")),
        Err(AppError {
            kind: ErrorKind::ResourceLimit,
            ..
        })
    ));
    queued.cancel();
    std::fs::write(h.barrier.join("release"), "").unwrap();
    assert!(first.wait().await.unwrap().error.is_none());
    let result = queued.wait().await.unwrap();
    assert_eq!(result.error.unwrap().kind, ErrorKind::Cancelled);
    assert!(result.provenance.runs.is_empty());
    h.worker.shutdown().await.unwrap();
    reaped(&h.barrier);
}
#[tokio::test]
async fn timeout_finalizes_and_reaps() {
    let h = Harness::start(
        "blocked",
        Limits {
            operation_timeout: Duration::from_millis(200),
            ..Limits::default()
        },
    )
    .await;
    let result = h.submit(command("fixture")).unwrap().wait().await.unwrap();
    assert_eq!(result.error.unwrap().kind, ErrorKind::Timeout);
    assert_eq!(result.provenance.runs.len(), 1);
    reaped(&h.barrier);
    h.worker.shutdown().await.unwrap();
}
#[tokio::test]
async fn source_failure_preserves_healthy_process_and_retry_metadata() {
    let h = Harness::start("source_error", Limits::default()).await;
    for _ in 0..2 {
        let result = h.submit(command("fixture")).unwrap().wait().await.unwrap();
        let error = result.error.unwrap();
        assert_eq!(error.kind, ErrorKind::RateLimited);
        assert_eq!(error.retry_after_seconds, Some(3));
        assert_eq!(result.provenance.runs.len(), 1);
        assert!(h.catalog.lock().unwrap().get("fixture").unwrap().available);
    }
    h.worker.shutdown().await.unwrap();
    reaped(&h.barrier);
}
#[tokio::test]
async fn page_item_and_byte_budgets_reject_before_commit() {
    for (mode, limits, retained) in [
        (
            "two_pages",
            Limits {
                max_pages_per_fetch: 1,
                ..Limits::default()
            },
            1,
        ),
        (
            "many_items",
            Limits {
                max_items_per_fetch: 2,
                max_read_page_items: 2,
                ..Limits::default()
            },
            0,
        ),
        (
            "large_bytes",
            Limits {
                max_bytes_per_fetch: 4096,
                max_document_bytes: 4096,
                ..Limits::default()
            },
            0,
        ),
    ] {
        let h = Harness::start(mode, limits).await;
        let result = h.submit(command("fixture")).unwrap().wait().await.unwrap();
        assert_eq!(
            result.error.unwrap().kind,
            ErrorKind::ResourceLimit,
            "{mode}"
        );
        let FetchCommand::Facts { query, .. } = command("fixture") else {
            unreachable!()
        };
        let snapshot = SqliteRepository::open(&h.db)
            .unwrap()
            .snapshot(&result.provenance.provider, &query)
            .unwrap();
        assert_eq!(snapshot.facts.len(), retained);
        assert_eq!(snapshot.runs[0].status, RunStatus::Failed);
        h.worker.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn shutdown_cancels_running_and_queued_then_reaps() {
    let h = Harness::start("blocked", Limits::default()).await;
    let first = h.submit(command("fixture")).unwrap();
    barrier(&h.barrier, "first").await;
    let queued = h.submit(command("fixture")).unwrap();
    h.worker.shutdown().await.unwrap();
    assert_eq!(
        first.wait().await.unwrap().error.unwrap().kind,
        ErrorKind::Cancelled
    );
    assert_eq!(
        queued.wait().await.unwrap().error.unwrap().kind,
        ErrorKind::Cancelled
    );
    reaped(&h.barrier);
    assert!(h.submit(command("fixture")).is_err());
}

#[tokio::test]
async fn queued_authorization_rechecks_activation_and_generation() {
    for restart in [false, true] {
        let h = Harness::start("blocked", Limits::default()).await;
        let first = h.submit(command("fixture")).unwrap();
        barrier(&h.barrier, "first").await;
        let queued = h.submit(command("fixture")).unwrap();
        {
            let mut catalog = h.catalog.lock().unwrap();
            if restart {
                let state = catalog.get("fixture").unwrap().clone();
                catalog.restart(state.identity, state.capabilities).unwrap();
            } else {
                catalog.deactivate("fixture").unwrap();
            }
        }
        std::fs::write(h.barrier.join("release"), "").unwrap();
        assert!(first.wait().await.unwrap().error.is_none());
        let result = queued.wait().await.unwrap();
        assert_eq!(
            result.error.unwrap().kind,
            if restart {
                ErrorKind::StaleReference
            } else {
                ErrorKind::Deactivated
            }
        );
        assert!(result.provenance.runs.is_empty());
        h.worker.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn resolution_failure_is_application_failure_with_exact_run() {
    let h = Harness::start("source_error", Limits::default()).await;
    let result = h
        .submit(FetchCommand::Resolve {
            instance_id: "fixture".into(),
            input: "AAPL".into(),
        })
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(result.error.unwrap().kind, ErrorKind::RateLimited);
    assert_eq!(result.provenance.runs[0].kind, RunKind::Resolution);
    h.worker.shutdown().await.unwrap();
}
#[tokio::test]
async fn resolution_fallback_shares_page_budget() {
    let h = Harness::start(
        "ok",
        Limits {
            max_pages_per_fetch: 1,
            ..Limits::default()
        },
    )
    .await;
    let result = h
        .submit(FetchCommand::Resolve {
            instance_id: "fixture".into(),
            input: "Apple".into(),
        })
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(result.error.unwrap().kind, ErrorKind::ResourceLimit);
    assert_eq!(result.provenance.runs.len(), 2);
    h.worker.shutdown().await.unwrap();
}
#[tokio::test]
async fn document_receipt_uses_actual_retrieval_time_and_checksum() {
    let h = Harness::start("ok", Limits::default()).await;
    let result = h
        .submit(FetchCommand::Document {
            instance_id: "fixture".into(),
            source_url: "https://example.test/document".into(),
        })
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(result.error.is_none());
    let document = result.provenance.document.unwrap();
    assert_eq!(
        document.retrieved_at.to_rfc3339(),
        "2026-09-09T00:00:00+00:00"
    );
    assert_eq!(
        SqliteRepository::open(&h.db)
            .unwrap()
            .stored_document(&document.checksum)
            .unwrap(),
        b"fixture document"
    );
    assert!(result.provenance.runs.is_empty());
    h.worker.shutdown().await.unwrap();
}
#[tokio::test]
async fn document_size_bound_is_resource_failure() {
    let h = Harness::start(
        "ok",
        Limits {
            max_document_bytes: 1,
            ..Limits::default()
        },
    )
    .await;
    let result = h
        .submit(FetchCommand::Document {
            instance_id: "fixture".into(),
            source_url: "https://example.test/document".into(),
        })
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(result.error.unwrap().kind, ErrorKind::ResourceLimit);
    assert!(result.provenance.document.is_none());
    h.worker.shutdown().await.unwrap();
}
#[tokio::test]
async fn startup_failure_reaps_child_and_never_advertises_available() {
    let root = tempfile::tempdir().unwrap();
    let factory = SqliteRepositoryFactory::new(root.path().join("db"));
    factory.initialize().unwrap();
    let provider = ProcessProviderFactory {
        manifest: Manifest {
            id: "worker-fixture".into(),
            version: "1".into(),
            protocol_version: 1,
            command: "python3".into(),
            args: vec![format!(
                "{}/tests/fixtures/worker.py",
                env!("CARGO_MANIFEST_DIR")
            )],
        },
        directory: root.path().to_path_buf(),
        instance_id: "fixture".into(),
        config: json!({"mode":"startup_failure", "barrier":root.path()}),
    };
    let catalog = Arc::new(Mutex::new(
        Catalog::new(vec![ProviderEntry {
            identity: provider.identity(),
            active: true,
            available: false,
            capabilities: Default::default(),
        }])
        .unwrap(),
    ));
    assert!(
        WorkerHandle::start(
            Arc::new(provider),
            Arc::new(factory),
            catalog.clone(),
            Limits::default(),
            WorkerSlots::new(1).unwrap()
        )
        .await
        .is_err()
    );
    assert!(!catalog.lock().unwrap().get("fixture").unwrap().available);
    reaped(root.path());
}

#[test]
fn global_worker_slots_require_finite_positive_capacity() {
    assert!(WorkerSlots::new(0).is_err());
    assert!(WorkerSlots::new(Limits::MAX_CONCURRENT_JOBS + 1).is_err());
}

#[tokio::test]
async fn subscribed_state_survives_consuming_wait_and_late_cancel_preserves_success() {
    let h = Harness::start("blocked", Limits::default()).await;
    let job = h.submit(command("fixture")).unwrap();
    let mut state = job.subscribe_state();
    let cancellation = job.cancellation();
    let result = tokio::spawn(job.wait());
    barrier(&h.barrier, "first").await;
    assert_eq!(*state.borrow_and_update(), JobState::Running);
    std::fs::write(h.barrier.join("release"), "").unwrap();
    let fetched = result.await.unwrap().unwrap();
    cancellation.cancel();
    assert_eq!(*state.borrow(), JobState::Succeeded);
    assert!(fetched.error.is_none());
    h.worker.shutdown().await.unwrap();
}
#[tokio::test]
async fn global_slots_bound_execution_and_waiting_worker_can_shutdown() {
    let slots = WorkerSlots::new(1).unwrap();
    let a = Harness::start_shared("blocked", Limits::default(), slots.clone()).await;
    let b = Harness::start_shared("ok", Limits::default(), slots).await;
    let first = a.submit(command("fixture")).unwrap();
    barrier(&a.barrier, "first").await;
    let second = b.submit(command("fixture")).unwrap();
    let mut state = second.subscribe_state();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), state.changed())
            .await
            .is_err()
    );
    assert!(!b.barrier.join("first").exists());
    tokio::time::timeout(Duration::from_secs(2), b.worker.shutdown())
        .await
        .unwrap()
        .unwrap();
    let result = second.wait().await.unwrap();
    assert_eq!(result.error.unwrap().kind, ErrorKind::Cancelled);
    assert!(result.provenance.runs.is_empty());
    first.cancel();
    first.wait().await.unwrap();
    a.worker.shutdown().await.unwrap();
}
#[tokio::test]
async fn finalization_storage_failure_outranks_cancellation() {
    let h = Harness::start("blocked", Limits::default()).await;
    let job = h.submit(command("fixture")).unwrap();
    barrier(&h.barrier, "first").await;
    let output = std::process::Command::new("python3").args(["-c", "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute(\"CREATE TRIGGER reject_finish BEFORE UPDATE ON runs BEGIN SELECT RAISE(FAIL, 'storage failure'); END\"); c.commit()"])
        .arg(&h.db).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    job.cancel();
    let result = job.wait().await.unwrap();
    assert_eq!(result.error.unwrap().kind, ErrorKind::Storage);
    assert_eq!(result.provenance.runs.len(), 1);
    reaped(&h.barrier);
    h.worker.shutdown().await.unwrap();
}
#[tokio::test]
async fn filings_prices_and_lookup_retain_typed_run_receipts() {
    let h = Harness::start("ok", Limits::default()).await;
    let FetchCommand::Facts { query, .. } = command("fixture") else {
        unreachable!()
    };
    let filings = h
        .submit(FetchCommand::Filings {
            instance_id: "fixture".into(),
            query,
        })
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(filings.error.is_none());
    assert_eq!(filings.provenance.runs[0].kind, RunKind::Financial);
    let price = h.submit(FetchCommand::Prices { instance_id: "fixture".into(), query: serde_json::from_value(json!({
        "instrument":{"namespace":"fixture:symbol","value":"AAPL"}, "start":"2024-01-01", "end":"2024-12-31", "page_size":10
    })).unwrap() }).unwrap().wait().await.unwrap();
    assert!(price.error.is_none(), "{:?}", price.error);
    assert_eq!(price.provenance.runs[0].kind, RunKind::Market);
    let lookup = h
        .submit(FetchCommand::Lookup {
            instance_id: "fixture".into(),
            request: serde_json::from_value(
                json!({"identifier":{"namespace":"sec:cik","value":"0000320193"}}),
            )
            .unwrap(),
        })
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(lookup.error.is_none(), "{:?}", lookup.error);
    assert_eq!(lookup.provenance.runs[0].kind, RunKind::Resolution);
    h.worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn resolution_bounds_historical_catalog_before_materialization() {
    let h = Harness::start(
        "ok",
        Limits {
            max_items_per_fetch: 1,
            max_read_page_items: 1,
            ..Limits::default()
        },
    )
    .await;
    let lookup = || FetchCommand::Lookup {
        instance_id: "fixture".into(),
        request: serde_json::from_value(
            json!({"identifier":{"namespace":"sec:cik","value":"0000320193"}}),
        )
        .unwrap(),
    };
    assert!(
        h.submit(lookup())
            .unwrap()
            .wait()
            .await
            .unwrap()
            .error
            .is_none()
    );
    let result = h.submit(lookup()).unwrap().wait().await.unwrap();
    assert_eq!(result.error.unwrap().kind, ErrorKind::ResourceLimit);
    assert_eq!(result.provenance.runs.len(), 1);
    assert!(h.catalog.lock().unwrap().get("fixture").unwrap().available);
    h.worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn rejected_finalization_is_storage_failure_even_without_sql_error() {
    let h = Harness::start("blocked", Limits::default()).await;
    let job = h.submit(command("fixture")).unwrap();
    barrier(&h.barrier, "first").await;
    let output = std::process::Command::new("python3")
        .args(["-c", "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute(\"UPDATE runs SET status='complete'\"); c.commit()"])
        .arg(&h.db).output().unwrap();
    assert!(output.status.success());
    job.cancel();
    let result = job.wait().await.unwrap();
    assert_eq!(result.error.unwrap().kind, ErrorKind::Storage);
    assert_eq!(result.provenance.runs.len(), 1);
    reaped(&h.barrier);
    h.worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn ingestion_protocol_error_reaps_provider_and_retains_committed_page() {
    let h = Harness::start("repeated_cursor", Limits::default()).await;
    let job = h.submit(command("fixture")).unwrap();
    barrier(&h.barrier, "second").await;
    std::fs::write(h.barrier.join("release"), "").unwrap();
    let result = job.wait().await.unwrap();
    let available = h.catalog.lock().unwrap().get("fixture").unwrap().available;
    let FetchCommand::Facts { query, .. } = command("fixture") else {
        unreachable!()
    };
    let snapshot = SqliteRepository::open(&h.db)
        .unwrap()
        .snapshot(&result.provenance.provider, &query)
        .unwrap();
    assert_eq!(result.error.unwrap().kind, ErrorKind::Unavailable);
    assert_eq!(snapshot.facts.len(), 1);
    assert_eq!(snapshot.runs[0].status, RunStatus::Failed);
    assert_eq!(snapshot.runs[0].id, result.provenance.runs[0].id);
    // Check the process before shutdown, but always clean up even on the RED path.
    let pid = std::fs::read_to_string(h.barrier.join("pid")).unwrap();
    let alive = std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .output()
        .unwrap()
        .status
        .success();
    h.worker.shutdown().await.unwrap();
    assert!(!available, "protocol-violating provider remained available");
    assert!(
        !alive,
        "protocol-violating child was not reaped before the result"
    );
}

#[tokio::test]
async fn ingestion_protocol_error_reaps_even_when_finalization_fails() {
    let h = Harness::start("repeated_cursor", Limits::default()).await;
    let job = h.submit(command("fixture")).unwrap();
    barrier(&h.barrier, "second").await;
    let output = std::process::Command::new("python3")
        .args(["-c", "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute(\"CREATE TRIGGER reject_finish BEFORE UPDATE ON runs BEGIN SELECT RAISE(FAIL, 'storage failure'); END\"); c.commit()"])
        .arg(&h.db).output().unwrap();
    assert!(output.status.success());
    std::fs::write(h.barrier.join("release"), "").unwrap();
    let result = job.wait().await.unwrap();
    let available = h.catalog.lock().unwrap().get("fixture").unwrap().available;
    assert_eq!(result.error.unwrap().kind, ErrorKind::Storage);
    assert_eq!(result.provenance.runs.len(), 1);
    let pid = std::fs::read_to_string(h.barrier.join("pid")).unwrap();
    let alive = std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .output()
        .unwrap()
        .status
        .success();
    h.worker.shutdown().await.unwrap();
    assert!(
        !available,
        "finalization failure hid required protocol invalidation"
    );
    assert!(!alive, "finalization failure prevented explicit reaping");
}

#[tokio::test]
async fn ingestion_market_protocol_error_reaps_before_returning() {
    let h = Harness::start("market_repeated_date", Limits::default()).await;
    let job = h.submit(FetchCommand::Prices { instance_id: "fixture".into(), query: serde_json::from_value(json!({
        "instrument":{"namespace":"fixture:symbol","value":"AAPL"}, "start":"2024-01-01", "end":"2024-12-31", "page_size":10
    })).unwrap() }).unwrap();
    barrier(&h.barrier, "second").await;
    std::fs::write(h.barrier.join("release"), "").unwrap();
    let result = job.wait().await.unwrap();
    let available = h.catalog.lock().unwrap().get("fixture").unwrap().available;
    assert_eq!(result.error.unwrap().kind, ErrorKind::Unavailable);
    assert_eq!(result.provenance.runs[0].kind, RunKind::Market);
    let pid = std::fs::read_to_string(h.barrier.join("pid")).unwrap();
    let alive = std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .output()
        .unwrap()
        .status
        .success();
    h.worker.shutdown().await.unwrap();
    assert!(
        !available,
        "cross-page market protocol violation remained available"
    );
    assert!(!alive, "market child was not reaped before terminal result");
}

#[path = "worker/startup_cleanup.rs"]
mod startup_cleanup;

#[tokio::test]
async fn resolution_failure_outranks_historical_read_limit() {
    for (mode, expected, cancel) in [
        ("search_blocked", ErrorKind::Cancelled, true),
        ("search_blocked", ErrorKind::Timeout, false),
        ("search_rate_limited", ErrorKind::RateLimited, false),
    ] {
        let h = Harness::start(
            mode,
            Limits {
                max_items_per_fetch: 1,
                max_read_page_items: 1,
                operation_timeout: Duration::from_secs(1),
                ..Limits::default()
            },
        )
        .await;
        // Two legitimate retrievals exceed the conservative catalog read budget.
        for _ in 0..2 {
            h.submit(FetchCommand::Lookup {
                instance_id: "fixture".into(),
                request: serde_json::from_value(json!({"identifier": {
                    "namespace": "sec:cik", "value": "0000320193"
                }}))
                .unwrap(),
            })
            .unwrap()
            .wait()
            .await
            .unwrap();
        }
        std::fs::remove_file(h.barrier.join("first")).unwrap();
        let job = h
            .submit(FetchCommand::Resolve {
                instance_id: "fixture".into(),
                input: "$AAPL".into(),
            })
            .unwrap();
        barrier(&h.barrier, "first").await;
        if cancel {
            job.cancel();
        }
        let result = job.wait().await.unwrap();
        if expected != ErrorKind::RateLimited {
            reaped(&h.barrier);
        }
        h.worker.shutdown().await.unwrap();
        let error = result.error.unwrap();
        assert_eq!(error.kind, expected);
        if expected == ErrorKind::RateLimited {
            assert_eq!(error.retry_after_seconds, Some(3));
        }
        assert_eq!(result.provenance.runs.len(), 1);
        let connection = rusqlite::Connection::open(&h.db).unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM resolution_runs WHERE id=?1",
                [result.provenance.runs[0].id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "failed");
    }
}
fn instrument_command() -> FetchCommand {
    FetchCommand::InstrumentLookup {
        instance_id: "fixture".into(),
        query: lugus_financial::instruments::InstrumentLookup {
            instrument: lugus_financial::market_data::InstrumentId {
                namespace: "yahoo:symbol".into(),
                value: "AAPL".into(),
            },
        },
    }
}
#[tokio::test]
async fn instrument_lookup_records_one_exact_observation_without_a_run() {
    let h = Harness::start(
        "ok",
        Limits {
            max_items_per_fetch: 1,
            max_read_page_items: 1,
            ..Limits::default()
        },
    )
    .await;
    let r = h
        .submit(instrument_command())
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(r.provenance.runs.is_empty());
    let o = r.provenance.instrument_observation.unwrap();
    assert_eq!(o.metadata.issuer_name.as_deref(), Some("Apple Inc."));
    let repo = SqliteRepository::open(&h.db).unwrap();
    assert_eq!(
        repo.bounded_instrument_observation(
            &o.provider,
            o.id,
            lugus_financial::storage::bounded::ReadLimits {
                max_items: 1,
                max_bytes: 10000
            }
        )
        .unwrap(),
        o
    );
    h.worker.shutdown().await.unwrap();
}
#[tokio::test]
async fn instrument_lookup_failure_budget_and_cancel_do_not_fabricate_evidence() {
    for mode in ["source_error", "ok", "blocked"] {
        let mut limits = Limits::default();
        if mode == "ok" {
            limits.max_bytes_per_fetch = 100;
            limits.max_document_bytes = 100;
        }
        let h = Harness::start(mode, limits).await;
        let job = h.submit(instrument_command()).unwrap();
        if mode == "blocked" {
            barrier(&h.barrier, "first").await;
            job.cancel();
        }
        let r = job.wait().await.unwrap();
        assert_eq!(
            r.error.unwrap().kind,
            match mode {
                "source_error" => ErrorKind::RateLimited,
                "ok" => ErrorKind::ResourceLimit,
                _ => ErrorKind::Cancelled,
            }
        );
        assert!(r.provenance.runs.is_empty());
        assert!(r.provenance.instrument_observation.is_none());
        h.worker.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn instrument_lookup_rejects_unsupported_version_before_dispatch() {
    let h = Harness::start("instrument_unsupported", Limits::default()).await;
    assert!(matches!(h.submit(instrument_command()),Err(e) if e.kind==ErrorKind::Unsupported));
    assert!(!h.barrier.join("first").exists());
    h.worker.shutdown().await.unwrap();
}
