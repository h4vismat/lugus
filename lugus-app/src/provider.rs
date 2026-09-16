//! Provider lifecycle and repository construction ports. No runtime or SQL leaks to providers.
use crate::{Limits, ProviderIdentity};
use async_trait::async_trait;
use lugus_financial::{
    capabilities::{FilingsProvider, FundamentalsProvider, MarketDataProvider},
    error::Result as FinancialResult,
    plugin::{Manifest, Plugin},
    resolution::{
        CompanyResolutionProvider,
        catalog::{CatalogRepository, ResolutionOutcome},
    },
    storage::{
        Repository, SqliteRepository,
        bounded::{ReadLimits, ReadResult},
        market::MarketRepository,
    },
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[async_trait]
pub trait ManagedProvider:
    FilingsProvider
    + FundamentalsProvider
    + lugus_financial::capabilities::HistoricalPricesProvider
    + MarketDataProvider
    + CompanyResolutionProvider
    + lugus_financial::instruments::InstrumentProvider
{
    fn capabilities(&self) -> &BTreeMap<String, u32>;
    fn is_running(&self) -> bool;
    async fn close(&mut self) -> FinancialResult<()>;
}
#[async_trait]
impl ManagedProvider for Plugin {
    fn capabilities(&self) -> &BTreeMap<String, u32> {
        self.capabilities()
    }
    fn is_running(&self) -> bool {
        self.is_running()
    }
    async fn close(&mut self) -> FinancialResult<()> {
        self.close().await
    }
}

/// Startup must bound its handshake and explicitly reap a child on initialization failure.
#[async_trait]
pub trait ProviderFactory: Send + Sync {
    fn identity(&self) -> ProviderIdentity;
    async fn start(&self, limits: &Limits) -> FinancialResult<Box<dyn ManagedProvider>>;
}
#[derive(Clone)]
pub struct ProcessProviderFactory {
    pub manifest: Manifest,
    pub directory: PathBuf,
    pub instance_id: String,
    pub config: serde_json::Value,
}
#[async_trait]
impl ProviderFactory for ProcessProviderFactory {
    fn identity(&self) -> ProviderIdentity {
        ProviderIdentity {
            instance_id: self.instance_id.clone(),
            plugin_id: self.manifest.id.clone(),
            plugin_version: self.manifest.version.clone(),
        }
    }
    async fn start(&self, limits: &Limits) -> FinancialResult<Box<dyn ManagedProvider>> {
        Ok(Box::new(
            Plugin::start(
                self.manifest.clone(),
                self.directory.clone(),
                self.instance_id.clone(),
                {
                    let mut config = self.config.clone();
                    if limits.operation_timeout == std::time::Duration::MAX
                        && matches!(self.manifest.id.as_str(), "sec-edgar" | "yfinance")
                    {
                        if !config.is_object() {
                            config = serde_json::json!({});
                        }
                        config["unlimited_research"] = serde_json::json!(true);
                    }
                    config
                },
                lugus_financial::plugin::Limits {
                    timeout: limits.operation_timeout,
                    // Allow bounded envelope overhead; the per-job wrapper counts all decoded payloads.
                    max_response_bytes: limits.max_bytes_per_fetch.saturating_add(4096),
                },
            )
            .await?,
        ))
    }
}

pub trait WorkerRepository:
    Repository
    + lugus_financial::storage::history::HistoryRepository
    + MarketRepository
    + CatalogRepository
    + lugus_financial::instruments::InstrumentRepository
{
    fn repository_identity(&self) -> FinancialResult<String>;
    fn bounded_resolution_outcome(
        &self,
        run: i64,
        limits: ReadLimits,
    ) -> ReadResult<ResolutionOutcome>;
}
impl WorkerRepository for SqliteRepository {
    fn repository_identity(&self) -> FinancialResult<String> {
        self.repository_identity()
    }
    fn bounded_resolution_outcome(
        &self,
        run: i64,
        limits: ReadLimits,
    ) -> ReadResult<ResolutionOutcome> {
        self.bounded_resolution_outcome(run, limits)
    }
}
/// `initialize` is called once before concurrent worker starts. `open` runs on the worker thread.
pub trait RepositoryFactory: Send + Sync {
    fn initialize(&self) -> FinancialResult<()>;
    fn open(&self) -> FinancialResult<Box<dyn WorkerRepository>>;
}
#[derive(Clone)]
pub struct SqliteRepositoryFactory {
    path: PathBuf,
}
impl SqliteRepositoryFactory {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}
impl RepositoryFactory for SqliteRepositoryFactory {
    fn initialize(&self) -> FinancialResult<()> {
        SqliteRepository::open(&self.path).map(|_| ())
    }
    fn open(&self) -> FinancialResult<Box<dyn WorkerRepository>> {
        Ok(Box::new(SqliteRepository::open(&self.path)?))
    }
}
