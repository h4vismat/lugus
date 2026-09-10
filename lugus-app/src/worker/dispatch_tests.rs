use super::*;
use crate::{ProcessProviderFactory, ProviderEntry, SqliteRepositoryFactory};
use lugus_financial::plugin::Manifest;
use serde_json::json;

#[tokio::test]
async fn cancellation_after_slot_acquisition_wins_at_catalog_dispatch_boundary() {
    let root = tempfile::tempdir().unwrap();
    let repositories = Arc::new(SqliteRepositoryFactory::new(
        root.path().join("financial.sqlite"),
    ));
    repositories.initialize().unwrap();
    let factory = ProcessProviderFactory {
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
        directory: root.path().into(),
        instance_id: "one".into(),
        config: json!({"mode":"ok","barrier":root.path().join("barrier")}),
    };
    let catalog = Arc::new(Mutex::new(
        Catalog::new(vec![ProviderEntry {
            identity: factory.identity(),
            active: true,
            available: false,
            capabilities: Default::default(),
        }])
        .unwrap(),
    ));
    let slots = WorkerSlots::new(1).unwrap();
    let hold_slot = slots.semaphore.clone().acquire_owned().await.unwrap();
    let worker = WorkerHandle::start(
        Arc::new(factory),
        repositories,
        catalog.clone(),
        Limits::default(),
        slots.clone(),
    )
    .await
    .unwrap();
    let offering = catalog.lock().unwrap().snapshot();
    let job = worker
        .submit(
            offering,
            Scope {
                workspace_id: "workspace".into(),
                request_id: "dispatch".into(),
                run_id: None,
            },
            FetchCommand::Document {
                instance_id: "one".into(),
                source_url: "https://example.test/document".into(),
            },
        )
        .unwrap();
    {
        let mut catalog = catalog.lock().unwrap();
        drop(hold_slot);
        // A real worker must acquire the released slot before we close catalog admission.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while slots.semaphore.available_permits() != 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "worker did not acquire capacity"
            );
            std::thread::yield_now();
        }
        catalog.deactivate("one").unwrap();
        job.cancel();
    }
    let result = job.wait().await.unwrap();
    assert_eq!(result.error.unwrap().kind, ErrorKind::Cancelled);
    assert!(result.provenance.runs.is_empty());
    assert!(result.provenance.document.is_none());
    assert!(!root.path().join("barrier/first").exists());
    worker.shutdown().await.unwrap();
}
