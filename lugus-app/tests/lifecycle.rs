//! Real child processes with gates only around startup/close to control cancellation races.
use async_trait::async_trait;
use lugus_app::*;
use lugus_financial::{
    capabilities::{FilingsProvider, FundamentalsProvider, MarketDataProvider, Provider},
    domain::{Document, Fact, Filing, Page},
    error::Result as FinancialResult,
    market_data::PricePage,
    plugin::Manifest,
    resolution::{Candidate, CompanyResolutionProvider, ResolutionPage, SearchRequest},
    storage::SqliteRepository,
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::Notify;
#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
    fail_close: bool,
}
struct Factory {
    inner: ProcessProviderFactory,
    start: Option<Arc<Gate>>,
    close: Option<Arc<Gate>>,
}
#[async_trait]
impl ProviderFactory for Factory {
    fn identity(&self) -> ProviderIdentity {
        self.inner.identity()
    }
    async fn start(&self, limits: &Limits) -> FinancialResult<Box<dyn ManagedProvider>> {
        if let Some(gate) = &self.start {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        Ok(Box::new(Peer {
            inner: self.inner.start(limits).await?,
            close: self.close.clone(),
        }))
    }
}
struct Peer {
    inner: Box<dyn ManagedProvider>,
    close: Option<Arc<Gate>>,
}
impl Provider for Peer {
    fn identity(&self) -> &ProviderIdentity {
        self.inner.identity()
    }
}
#[async_trait]
impl ManagedProvider for Peer {
    fn capabilities(&self) -> &BTreeMap<String, u32> {
        self.inner.capabilities()
    }
    fn is_running(&self) -> bool {
        self.inner.is_running()
    }
    async fn close(&mut self) -> FinancialResult<()> {
        let fail = if let Some(gate) = self.close.take() {
            gate.entered.notify_one();
            gate.release.notified().await;
            gate.fail_close
        } else {
            false
        };
        self.inner.close().await?;
        if fail {
            Err(lugus_financial::error::Error::new(
                lugus_financial::error::ErrorKind::Timeout,
                "injected cleanup failure",
            ))
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl FilingsProvider for Peer {
    async fn list_filings(&mut self, q: &Query) -> FinancialResult<Page<Filing>> {
        self.inner.list_filings(q).await
    }
    async fn fetch_document(&mut self, url: &str, max: usize) -> FinancialResult<Document> {
        self.inner.fetch_document(url, max).await
    }
}
#[async_trait]
impl FundamentalsProvider for Peer {
    async fn fetch_facts(&mut self, q: &Query) -> FinancialResult<Page<Fact>> {
        self.inner.fetch_facts(q).await
    }
}
#[async_trait]
impl MarketDataProvider for Peer {
    async fn fetch_prices(&mut self, q: &PriceQuery) -> FinancialResult<PricePage> {
        self.inner.fetch_prices(q).await
    }
}
#[async_trait]
impl CompanyResolutionProvider for Peer {
    async fn search_companies(&mut self, q: &SearchRequest) -> FinancialResult<ResolutionPage> {
        self.inner.search_companies(q).await
    }
    async fn lookup_company(&mut self, q: &LookupRequest) -> FinancialResult<Candidate> {
        self.inner.lookup_company(q).await
    }
}
async fn host(
    start: Option<Arc<Gate>>,
    close: Option<Arc<Gate>>,
) -> (tempfile::TempDir, Application) {
    let root = tempfile::tempdir().unwrap();
    let active = start.is_none();
    let app = app_at(root.path(), start, close, active).await;
    (root, app)
}
async fn app_at(
    root: &std::path::Path,
    start: Option<Arc<Gate>>,
    close: Option<Arc<Gate>>,
    active: bool,
) -> Application {
    let financial = root.join("financial.sqlite");
    let repo = Arc::new(SqliteRepositoryFactory::new(&financial));
    repo.initialize().unwrap();
    let store = SqliteApplicationStore::open(
        root.join("application.sqlite"),
        Box::new(SqliteRepository::open(&financial).unwrap()),
        Limits::default(),
        Box::new(SystemClock),
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap();
    let factory = Factory {
        inner: ProcessProviderFactory {
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
            directory: root.into(),
            instance_id: "one".into(),
            config: serde_json::json!({"mode":"ok","barrier":root.join("one")}),
        },
        start,
        close,
    };
    Application::start(
        vec![ConfiguredProvider {
            active,
            factory: Arc::new(factory),
        }],
        repo,
        Box::new(store),
        Limits::default(),
        HostBounds::default(),
        Box::new(RandomIds::new().unwrap()),
    )
    .await
    .unwrap()
}
async fn reaped(root: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(pid) = std::fs::read_to_string(root.join("one/pid"))
                && !std::process::Command::new("kill")
                    .args(["-0", pid.trim()])
                    .output()
                    .unwrap()
                    .status
                    .success()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn dropped_activation_remains_owned_until_shutdown_finishes() {
    let gate = Arc::new(Gate::default());
    let (root, app) = host(Some(gate.clone()), None).await;
    let activate = {
        let app = app.clone();
        tokio::spawn(async move { app.activate("one").await })
    };
    gate.entered.notified().await;
    activate.abort();
    assert!(activate.await.unwrap_err().is_cancelled());
    let early = tokio::time::timeout(Duration::from_millis(100), app.shutdown())
        .await
        .is_ok();
    gate.release.notify_one();
    app.shutdown().await.unwrap();
    reaped(root.path()).await;
    assert!(!early, "shutdown returned before admitted startup finished");
    assert!(!app.providers().unwrap()[0].available);
}
#[tokio::test]
async fn dropped_deactivation_remains_owned_until_shutdown_finishes() {
    let gate = Arc::new(Gate::default());
    let (root, app) = host(None, Some(gate.clone())).await;
    let deactivate = {
        let app = app.clone();
        tokio::spawn(async move { app.deactivate("one").await })
    };
    gate.entered.notified().await;
    deactivate.abort();
    assert!(deactivate.await.unwrap_err().is_cancelled());
    let early = tokio::time::timeout(Duration::from_millis(100), app.shutdown())
        .await
        .is_ok();
    gate.release.notify_one();
    app.shutdown().await.unwrap();
    reaped(root.path()).await;
    assert!(
        !early,
        "shutdown returned before admitted deactivation reaped its child"
    );
}
#[tokio::test]
async fn dropped_shutdown_waiter_does_not_drop_cleanup() {
    let gate = Arc::new(Gate::default());
    let (root, app) = host(None, Some(gate.clone())).await;
    let shutdown = {
        let app = app.clone();
        tokio::spawn(async move { app.shutdown().await })
    };
    gate.entered.notified().await;
    shutdown.abort();
    assert!(shutdown.await.unwrap_err().is_cancelled());
    let early = tokio::time::timeout(Duration::from_millis(100), app.shutdown())
        .await
        .is_ok();
    gate.release.notify_one();
    app.shutdown().await.unwrap();
    reaped(root.path()).await;
    assert!(!early, "a dropped shutdown caller lost its cleanup task");
}

#[tokio::test]
async fn dropped_host_construction_reaps_its_admitted_startup() {
    let root = tempfile::tempdir().unwrap();
    let gate = Arc::new(Gate::default());
    let constructing = {
        let path = root.path().to_path_buf();
        let gate = gate.clone();
        tokio::spawn(async move { app_at(&path, Some(gate), None, true).await })
    };
    gate.entered.notified().await;
    constructing.abort();
    assert!(constructing.await.err().unwrap().is_cancelled());
    gate.release.notify_one();
    reaped(root.path()).await;
}
#[tokio::test]
async fn dropped_deactivation_preserves_cleanup_failure_for_shutdown() {
    let gate = Arc::new(Gate {
        fail_close: true,
        ..Gate::default()
    });
    let (root, app) = host(None, Some(gate.clone())).await;
    let deactivate = {
        let app = app.clone();
        tokio::spawn(async move { app.deactivate("one").await })
    };
    gate.entered.notified().await;
    deactivate.abort();
    assert!(deactivate.await.unwrap_err().is_cancelled());
    gate.release.notify_one();
    assert_eq!(app.shutdown().await.unwrap_err().kind, ErrorKind::Timeout);
    assert_eq!(app.shutdown().await.unwrap_err().kind, ErrorKind::Timeout);
    reaped(root.path()).await;
}

#[tokio::test]
async fn startup_cleanup_failure_after_shutdown_fence_is_not_lost() {
    let start = Arc::new(Gate::default());
    let close = Arc::new(Gate {
        fail_close: true,
        ..Gate::default()
    });
    let (root, app) = host(Some(start.clone()), Some(close.clone())).await;
    let activate = {
        let app = app.clone();
        tokio::spawn(async move { app.activate("one").await })
    };
    start.entered.notified().await;
    activate.abort();
    assert!(activate.await.unwrap_err().is_cancelled());
    let shutdown = {
        let app = app.clone();
        tokio::spawn(async move { app.shutdown().await })
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        while app.providers().unwrap()[0].active {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    start.release.notify_one();
    close.entered.notified().await;
    close.release.notify_one();
    assert_eq!(
        shutdown.await.unwrap().unwrap_err().kind,
        ErrorKind::Timeout
    );
    reaped(root.path()).await;
}
