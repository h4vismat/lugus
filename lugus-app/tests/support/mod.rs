#![allow(dead_code)]
use lugus_app::*;
use lugus_financial::{plugin::Manifest, storage::SqliteRepository};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub struct Harness {
    pub root: tempfile::TempDir,
    pub app: Application,
    pub financial: PathBuf,
    pub application: PathBuf,
}
impl Harness {
    pub async fn new(modes: &[(&str, &str)], bounds: HostBounds, limits: Limits) -> Self {
        let root = tempfile::tempdir().unwrap();
        let financial = root.path().join("financial.sqlite");
        let application = root.path().join("application.sqlite");
        let repositories = Arc::new(SqliteRepositoryFactory::new(&financial));
        repositories.initialize().unwrap();
        let store = SqliteApplicationStore::open(
            &application,
            Box::new(SqliteRepository::open(&financial).unwrap()),
            limits.clone(),
            Box::new(SystemClock),
            Box::new(RandomIds::new().unwrap()),
        )
        .unwrap();
        let providers = modes
            .iter()
            .map(|(id, mode)| ConfiguredProvider {
                active: true,
                factory: Arc::new(ProcessProviderFactory {
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
                    instance_id: (*id).into(),
                    config: json!({"mode":mode,"barrier":root.path().join(id)}),
                }),
            })
            .collect();
        let app = Application::start(
            providers,
            repositories,
            Box::new(store),
            limits,
            bounds,
            Box::new(RandomIds::new().unwrap()),
        )
        .await
        .unwrap();
        Self {
            root,
            app,
            financial,
            application,
        }
    }
    pub fn scope(&self, request: &str) -> Scope {
        self.app.scope("workspace", request, None).unwrap()
    }
    pub async fn barrier(&self, id: &str, name: &str) {
        barrier(&self.root.path().join(id), name).await;
    }
}
pub async fn barrier(path: &Path, name: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.join(name).exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("fixture barrier");
}
pub fn command(id: &str) -> FetchCommand {
    FetchCommand::Filings {
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
pub fn projection(fetch: &FetchReference) -> DatasetProjection {
    DatasetProjection::Filings {
        run_id: fetch.runs[0].id,
    }
}
