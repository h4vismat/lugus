mod support;
use lugus_app::*;
use support::*;

#[tokio::test]
async fn manual_fetch_has_durable_scoped_reference_and_offline_view() {
    let h = Harness::new(
        &[("one", "ok"), ("bad", "startup_failure")],
        HostBounds::default(),
        Limits::default(),
    )
    .await;
    let scope = h.scope("manual");
    let job = h.app.submit_manual(&scope, command("one")).unwrap();
    let terminal = h.app.wait(&scope, &job.id).await.unwrap();
    assert_eq!(terminal.state, JobState::Succeeded);
    let fetch = h
        .app
        .read_fetch(&scope, terminal.fetch_id.as_ref().unwrap())
        .await
        .unwrap();
    assert_eq!(fetch.provider.instance_id, "one");
    assert_eq!(fetch.scope, scope);
    assert_eq!(
        h.app
            .submit_manual(&scope, command("bad"))
            .unwrap_err()
            .kind,
        ErrorKind::Unavailable
    );
    h.app.shutdown().await.unwrap();
    let dataset = h
        .app
        .create_dataset(&scope, &fetch.id, projection(&fetch))
        .await
        .unwrap();
    let page = h
        .app
        .read_dataset(
            &scope,
            &dataset.id,
            PageRequest {
                offset: 0,
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    let receipt = h
        .app
        .open_view(
            &scope,
            OpenViewRequest {
                dataset_id: dataset.id.clone(),
                kind: ViewKind::DataTable,
            },
        )
        .await
        .unwrap();
    assert!(receipt.presentation.is_none());
    let other = h.app.scope("other", "manual", None).unwrap();
    assert_eq!(
        h.app.status(&other, &job.id).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    assert_eq!(
        h.app.cancel(&other, &job.id).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    assert_eq!(
        h.app
            .read_dataset(
                &other,
                &dataset.id,
                PageRequest {
                    offset: 0,
                    limit: 1
                }
            )
            .await
            .unwrap_err()
            .kind,
        ErrorKind::ScopeMismatch
    );
    assert_eq!(
        h.app
            .report_presentation(
                &other,
                PresentationResult {
                    view_id: receipt.id,
                    descriptor_revision: 1,
                    status: PresentationStatus::Presented
                }
            )
            .await
            .unwrap_err()
            .kind,
        ErrorKind::ScopeMismatch
    );
    assert_eq!(
        h.app
            .submit_manual(&scope, command("one"))
            .unwrap_err()
            .kind,
        ErrorKind::Unavailable
    );
}

#[tokio::test]
async fn deactivation_fences_queued_jobs_and_restart_invalidates_turn() {
    let h = Harness::new(
        &[("one", "blocked"), ("two", "ok")],
        HostBounds::default(),
        Limits {
            max_concurrent_jobs: 2,
            ..Limits::default()
        },
    )
    .await;
    let scope = h.scope("request");
    let offering = h.app.offering().unwrap();
    let first = h.app.submit(&scope, &offering, command("one")).unwrap();
    h.barrier("one", "first").await;
    let queued = h.app.submit(&scope, &offering, command("one")).unwrap();
    let second = h.app.submit_manual(&scope, command("two")).unwrap();
    assert_eq!(
        h.app.wait(&scope, &second.id).await.unwrap().state,
        JobState::Succeeded
    );
    h.app.deactivate("one").await.unwrap();
    for id in [&first.id, &queued.id] {
        assert_eq!(
            h.app.wait(&scope, id).await.unwrap().state,
            JobState::Cancelled
        );
    }
    assert_eq!(
        h.app
            .submit(&scope, &offering, command("one"))
            .unwrap_err()
            .kind,
        ErrorKind::Deactivated
    );
    h.app.activate("one").await.unwrap();
    assert_eq!(
        h.app
            .submit(&scope, &offering, command("one"))
            .unwrap_err()
            .kind,
        ErrorKind::StaleReference
    );
    h.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn registry_is_bounded_and_backpressure_does_not_block_terminal_cleanup() {
    let h = Harness::new(
        &[("one", "blocked")],
        HostBounds {
            max_pending_jobs: 1,
            max_terminal_jobs: 1,
            event_capacity: 1,
        },
        Limits::default(),
    )
    .await;
    let scope = h.scope("request");
    let _unread_events = h.app.subscribe();
    let first = h.app.submit_manual(&scope, command("one")).unwrap();
    h.barrier("one", "first").await;
    assert_eq!(
        h.app
            .submit_manual(&scope, command("one"))
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    h.app.cancel(&scope, &first.id).unwrap();
    assert_eq!(
        h.app.wait(&scope, &first.id).await.unwrap().state,
        JobState::Cancelled
    );
    let durable = h.app.status(&scope, &first.id).unwrap().fetch_id.unwrap();
    h.app.activate("one").await.unwrap();
    let second = h.app.submit_manual(&scope, command("one")).unwrap();
    h.app.shutdown().await.unwrap();
    let terminal = h.app.wait(&scope, &second.id).await.unwrap();
    assert_eq!(terminal.state, JobState::Cancelled, "{terminal:?}");
    assert_eq!(
        h.app.status(&scope, &first.id).unwrap_err().kind,
        ErrorKind::MissingData
    );
    assert!(h.app.read_fetch(&scope, &durable).await.is_ok());
}

#[tokio::test]
async fn late_cancel_preserves_authoritative_success() {
    let h = Harness::new(&[("one", "ok")], HostBounds::default(), Limits::default()).await;
    let scope = h.scope("request");
    let job = h.app.submit_manual(&scope, command("one")).unwrap();
    assert_eq!(
        h.app.wait(&scope, &job.id).await.unwrap().state,
        JobState::Succeeded
    );
    h.app.cancel(&scope, &job.id).unwrap();
    assert_eq!(
        h.app.status(&scope, &job.id).unwrap().state,
        JobState::Succeeded
    );
    h.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn manual_local_inputs_reject_oversized_payloads_before_storage() {
    let h = Harness::new(
        &[],
        HostBounds::default(),
        Limits {
            max_input_bytes: 256,
            ..Limits::default()
        },
    )
    .await;
    let scope = h.scope("local");
    let mut query = match command("one") {
        FetchCommand::Filings { query, .. } => query,
        _ => unreachable!(),
    };
    query.forms = vec!["x".repeat(400)];
    let projection = DatasetProjection::Facts {
        run_id: 1,
        query: lugus_financial::selection::MetricQuery {
            scope: query,
            namespace: "us-gaap".into(),
            concept: "Assets".into(),
            unit: "USD".into(),
            periods: lugus_financial::selection::PeriodSelection::Instants,
        },
    };
    assert_eq!(
        h.app
            .create_dataset(&scope, "missing", projection)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        h.app.status(&scope, &"x".repeat(257)).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    h.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn application_persistence_failure_is_authoritative_failure() {
    let h = Harness::new(&[("one", "ok")], HostBounds::default(), Limits::default()).await;
    rusqlite::Connection::open(&h.application).unwrap().execute_batch("CREATE TRIGGER reject_fetch BEFORE INSERT ON app_records BEGIN SELECT RAISE(ABORT, 'storage secret'); END;").unwrap();
    let scope = h.scope("storage");
    let job = h.app.submit_manual(&scope, command("one")).unwrap();
    let result = h.app.wait(&scope, &job.id).await.unwrap();
    assert_eq!(result.state, JobState::Failed);
    assert_eq!(result.error.as_ref().unwrap().kind, ErrorKind::Storage);
    assert!(!result.error.unwrap().message.contains("secret"));
    assert!(result.fetch_id.is_none());
    h.app.shutdown().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn blocked_application_sql_does_not_block_async_execution() {
    let h = Harness::new(&[("one", "ok")], HostBounds::default(), Limits::default()).await;
    let scope = h.scope("sql");
    let job = h.app.submit_manual(&scope, command("one")).unwrap();
    let fetch = h.app.wait(&scope, &job.id).await.unwrap().fetch_id.unwrap();
    let connection = rusqlite::Connection::open(&h.application).unwrap();
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let app = h.app.clone();
    let s = scope.clone();
    let read = tokio::spawn(async move { app.read_fetch(&s, &fetch).await });
    // A blocking caller-thread SQL read would stall this single-thread runtime for its 5s busy timeout.
    let started = std::time::Instant::now();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    connection.execute_batch("COMMIT").unwrap();
    assert!(read.await.unwrap().is_ok());
    h.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn durable_fetch_reopens_in_new_offline_host() {
    let h = Harness::new(&[("one", "ok")], HostBounds::default(), Limits::default()).await;
    let scope = h.scope("reopen");
    let job = h.app.submit_manual(&scope, command("one")).unwrap();
    let fetch_id = h.app.wait(&scope, &job.id).await.unwrap().fetch_id.unwrap();
    h.app.shutdown().await.unwrap();
    let store = SqliteApplicationStore::open(
        &h.application,
        Box::new(lugus_financial::storage::SqliteRepository::open(&h.financial).unwrap()),
        Limits::default(),
        Box::new(SystemClock),
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap();
    let reopened = Application::start(
        vec![],
        std::sync::Arc::new(SqliteRepositoryFactory::new(&h.financial)),
        Box::new(store),
        Limits::default(),
        HostBounds::default(),
        Box::new(RandomIds::new().unwrap()),
    )
    .await
    .unwrap();
    let fetch = reopened.read_fetch(&scope, &fetch_id).await.unwrap();
    let dataset = reopened
        .create_dataset(&scope, &fetch_id, projection(&fetch))
        .await
        .unwrap();
    assert_eq!(dataset.row_count, 1);
    assert_eq!(
        reopened.status(&scope, &job.id).unwrap_err().kind,
        ErrorKind::MissingData
    );
    reopened.shutdown().await.unwrap();
}

struct GatedFactory {
    inner: ProcessProviderFactory,
    entered: std::sync::Arc<tokio::sync::Notify>,
    release: std::sync::Arc<tokio::sync::Notify>,
}
#[async_trait::async_trait]
impl ProviderFactory for GatedFactory {
    fn identity(&self) -> ProviderIdentity {
        self.inner.identity()
    }
    async fn start(
        &self,
        limits: &Limits,
    ) -> lugus_financial::error::Result<Box<dyn ManagedProvider>> {
        self.entered.notify_one();
        self.release.notified().await;
        self.inner.start(limits).await
    }
}
#[tokio::test]
async fn shutdown_fences_a_concurrent_startup_and_reaps_before_returning() {
    use std::sync::Arc;
    let root = tempfile::tempdir().unwrap();
    let financial = root.path().join("financial.sqlite");
    let repo = Arc::new(SqliteRepositoryFactory::new(&financial));
    repo.initialize().unwrap();
    let store = SqliteApplicationStore::open(
        root.path().join("app.sqlite"),
        Box::new(lugus_financial::storage::SqliteRepository::open(&financial).unwrap()),
        Limits::default(),
        Box::new(SystemClock),
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let factory = GatedFactory {
        entered: entered.clone(),
        release: release.clone(),
        inner: ProcessProviderFactory {
            manifest: lugus_financial::plugin::Manifest {
                id: "worker-fixture".into(),
                version: "1".into(),
                protocol_version: 1,
                command: "python3".into(),
                args: vec![format!(
                    "{}/tests/fixtures/worker.py",
                    env!("CARGO_MANIFEST_DIR")
                )],
            },
            directory: root.path().into(),
            instance_id: "one".into(),
            config: serde_json::json!({"mode":"ok","barrier":root.path().join("one")}),
        },
    };
    let app = Application::start(
        vec![ConfiguredProvider {
            active: false,
            factory: Arc::new(factory),
        }],
        repo,
        Box::new(store),
        Limits::default(),
        HostBounds::default(),
        Box::new(RandomIds::new().unwrap()),
    )
    .await
    .unwrap();
    let activating = {
        let app = app.clone();
        tokio::spawn(async move { app.activate("one").await })
    };
    entered.notified().await;
    let shutdown = {
        let app = app.clone();
        tokio::spawn(async move { app.shutdown().await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while app.providers().unwrap()[0].active {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let scope = app.scope("workspace", "shutdown", None).unwrap();
    assert_eq!(
        app.submit_manual(&scope, command("one")).unwrap_err().kind,
        ErrorKind::Unavailable
    );
    release.notify_one();
    assert_eq!(
        activating.await.unwrap().unwrap_err().kind,
        ErrorKind::Unavailable
    );
    shutdown.await.unwrap().unwrap();
    assert!(!app.providers().unwrap()[0].available);
    let pid = std::fs::read_to_string(root.path().join("one/pid")).unwrap();
    assert!(
        !std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[tokio::test]
async fn manual_fetch_read_respects_output_budget_without_losing_durable_job_receipt() {
    let h = Harness::new(
        &[("one", "ok")],
        HostBounds::default(),
        Limits {
            max_output_bytes: Limits::MIN_OUTPUT_BYTES,
            max_read_page_bytes: Limits::MIN_OUTPUT_BYTES,
            ..Limits::default()
        },
    )
    .await;
    let scope = h.scope("large-reference");
    let receipt = h
        .app
        .submit_manual(
            &scope,
            FetchCommand::Document {
                instance_id: "one".into(),
                source_url: format!("https://example.test/{}", "x".repeat(3000)),
            },
        )
        .unwrap();
    let terminal = h.app.wait(&scope, &receipt.id).await.unwrap();
    assert_eq!(terminal.state, JobState::Succeeded);
    assert_eq!(
        h.app
            .read_fetch(&scope, terminal.fetch_id.as_ref().unwrap())
            .await
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    h.app.shutdown().await.unwrap();
}

#[tokio::test]
async fn subscribed_waiter_keeps_completion_after_terminal_registry_eviction() {
    use std::{future::Future, task::Poll};
    let h = Harness::new(
        &[("one", "blocked")],
        HostBounds {
            max_pending_jobs: 1,
            max_terminal_jobs: 1,
            event_capacity: 1,
        },
        Limits::default(),
    )
    .await;
    let scope = h.scope("retention");
    let first = h.app.submit_manual(&scope, command("one")).unwrap();
    h.barrier("one", "first").await;
    let mut waiting = Box::pin(h.app.wait(&scope, &first.id));
    std::future::poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    h.app.cancel(&scope, &first.id).unwrap();
    assert_eq!(
        h.app.wait(&scope, &first.id).await.unwrap().state,
        JobState::Cancelled
    );
    h.app.activate("one").await.unwrap();
    std::fs::write(h.root.path().join("one/release"), "").unwrap();
    let second = h.app.submit_manual(&scope, command("one")).unwrap();
    assert_eq!(
        h.app.wait(&scope, &second.id).await.unwrap().state,
        JobState::Succeeded
    );
    assert_eq!(
        h.app.status(&scope, &first.id).unwrap_err().kind,
        ErrorKind::MissingData
    );
    assert_eq!(waiting.await.unwrap().state, JobState::Cancelled);
    h.app.shutdown().await.unwrap();
}
