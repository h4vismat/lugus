//! Inject only a cleanup failure; startup, child shutdown, and reaping remain real.
use super::*;
use async_trait::async_trait;
use lugus_financial::{
    capabilities::{FilingsProvider, FundamentalsProvider, MarketDataProvider, Provider},
    domain::{Document, Fact, Filing, Page},
    error::{Error, ErrorKind as FinancialKind, Result as FinancialResult},
    market_data::PricePage,
    resolution::{
        Candidate, CompanyResolutionProvider, LookupRequest, ResolutionPage, SearchRequest,
    },
};
use std::collections::BTreeMap;

struct RegistrationFailureFactory {
    inner: ProcessProviderFactory,
    cleanup_fails: bool,
}
#[async_trait]
impl ProviderFactory for RegistrationFailureFactory {
    fn identity(&self) -> ProviderIdentity {
        self.inner.identity()
    }
    async fn start(&self, limits: &Limits) -> FinancialResult<Box<dyn ManagedProvider>> {
        Ok(Box::new(RegistrationFailureProvider {
            inner: self.inner.start(limits).await?,
            // Registration rejects version zero after successful process startup.
            capabilities: BTreeMap::from([("filings".into(), 0)]),
            cleanup_fails: self.cleanup_fails,
        }))
    }
}
struct RegistrationFailureProvider {
    inner: Box<dyn ManagedProvider>,
    capabilities: BTreeMap<String, u32>,
    cleanup_fails: bool,
}
impl Provider for RegistrationFailureProvider {
    fn identity(&self) -> &ProviderIdentity {
        self.inner.identity()
    }
}
#[async_trait]
impl ManagedProvider for RegistrationFailureProvider {
    fn capabilities(&self) -> &BTreeMap<String, u32> {
        &self.capabilities
    }
    fn is_running(&self) -> bool {
        self.inner.is_running()
    }
    async fn close(&mut self) -> FinancialResult<()> {
        self.inner.close().await?;
        if self.cleanup_fails {
            Err(Error::new(
                FinancialKind::Timeout,
                "injected cleanup failure",
            ))
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl FilingsProvider for RegistrationFailureProvider {
    async fn list_filings(&mut self, query: &Query) -> FinancialResult<Page<Filing>> {
        self.inner.list_filings(query).await
    }
    async fn fetch_document(&mut self, url: &str, max: usize) -> FinancialResult<Document> {
        self.inner.fetch_document(url, max).await
    }
}
#[async_trait]
impl FundamentalsProvider for RegistrationFailureProvider {
    async fn fetch_facts(&mut self, query: &Query) -> FinancialResult<Page<Fact>> {
        self.inner.fetch_facts(query).await
    }
}
#[async_trait]
impl MarketDataProvider for RegistrationFailureProvider {
    async fn fetch_prices(&mut self, query: &PriceQuery) -> FinancialResult<PricePage> {
        self.inner.fetch_prices(query).await
    }
}
#[async_trait]
impl CompanyResolutionProvider for RegistrationFailureProvider {
    async fn search_companies(
        &mut self,
        request: &SearchRequest,
    ) -> FinancialResult<ResolutionPage> {
        self.inner.search_companies(request).await
    }
    async fn lookup_company(&mut self, request: &LookupRequest) -> FinancialResult<Candidate> {
        self.inner.lookup_company(request).await
    }
}

#[tokio::test]
async fn startup_registration_preserves_cleanup_failure_precedence() {
    for (cleanup_fails, expected) in [(false, ErrorKind::InvalidInput), (true, ErrorKind::Timeout)]
    {
        let root = tempfile::tempdir().unwrap();
        let repositories = SqliteRepositoryFactory::new(root.path().join("db"));
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
            directory: root.path().to_path_buf(),
            instance_id: "fixture".into(),
            config: json!({"mode": "ok", "barrier": root.path()}),
        };
        let catalog = Arc::new(Mutex::new(
            Catalog::new(vec![ProviderEntry {
                identity: factory.identity(),
                active: true,
                available: false,
                capabilities: BTreeMap::new(),
            }])
            .unwrap(),
        ));
        let result = WorkerHandle::start(
            Arc::new(RegistrationFailureFactory {
                inner: factory,
                cleanup_fails,
            }),
            Arc::new(repositories),
            catalog.clone(),
            Limits::default(),
            WorkerSlots::new(1).unwrap(),
        )
        .await;
        let error = match result {
            Err(error) => error,
            Ok(worker) => {
                worker.shutdown().await.unwrap();
                panic!("registration should fail");
            }
        };
        reaped(root.path());
        assert!(!catalog.lock().unwrap().get("fixture").unwrap().available);
        assert_eq!(error.kind, expected, "cleanup_fails={cleanup_fails}");
    }
}
