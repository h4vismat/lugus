#![allow(dead_code)]
use base64::{Engine, engine::general_purpose::STANDARD};
use lugus_app::*;
use lugus_financial::{
    domain::Document,
    storage::{Repository, SqliteRepository},
};
use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
pub struct Ids(pub AtomicU64);
impl IdSource for Ids {
    fn next_id(&self) -> String {
        format!("passage-{}", self.0.fetch_add(1, Ordering::Relaxed))
    }
}
pub fn scope() -> Scope {
    Scope {
        workspace_id: "workspace".into(),
        request_id: "request".into(),
        run_id: None,
    }
}
pub fn open(db: &Path, fin: &Path, seed: u64) -> SqliteApplicationStore {
    SqliteApplicationStore::open(
        db,
        Box::new(SqliteRepository::open(fin).unwrap()),
        Limits::default(),
        Box::new(SystemClock),
        Box::new(Ids(AtomicU64::new(seed))),
    )
    .unwrap()
}
pub fn dataset(store: &mut SqliteApplicationStore, fin: &Path, html: &str) -> DatasetHeader {
    dataset_scoped(store, fin, html, &scope())
}
pub fn dataset_scoped(
    store: &mut SqliteApplicationStore,
    fin: &Path,
    html: &str,
    scope: &Scope,
) -> DatasetHeader {
    let mut repo = SqliteRepository::open(fin).unwrap();
    let provider = ProviderIdentity {
        instance_id: "fixture".into(),
        plugin_id: "fixture".into(),
        plugin_version: "1".into(),
    };
    let doc = Document {
        source_url: "https://fixture.test/filing".into(),
        media_type: "text/html".into(),
        content_base64: STANDARD.encode(html),
        retrieved_at: "2026-09-10T00:00:00Z".parse().unwrap(),
    };
    let sum = repo
        .save_document(&provider, &doc, 32 * 1024 * 1024)
        .unwrap();
    let observation = repo.document_observations(&sum).unwrap().remove(0);
    let fetch = store
        .record_fetch(&FetchResult {
            provenance: FetchProvenance {
                scope: scope.clone(),
                provider,
                repository_id: repo.repository_identity().unwrap(),
                command: FetchCommand::Document {
                    instance_id: "fixture".into(),
                    source_url: doc.source_url,
                },
                runs: vec![],
                document: Some(observation),
                instrument_observation: None,
                binding_id: None,
            },
            error: None,
        })
        .unwrap();
    store
        .create_dataset(scope, &fetch.id, DatasetProjection::Document)
        .unwrap()
}

pub async fn application(store: SqliteApplicationStore, fin: &Path) -> Application {
    Application::start(
        vec![],
        std::sync::Arc::new(SqliteRepositoryFactory::new(fin)),
        Box::new(store),
        Limits::default(),
        HostBounds::default(),
        Box::new(Ids(AtomicU64::new(1000))),
    )
    .await
    .unwrap()
}
