//! Optional JSON/process/SQLite composition adapter; the core host depends only on ports.
use crate::*;
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessConfig {
    pub instance_id: String,
    pub manifest: PathBuf,
    pub active: bool,
    #[serde(default)]
    pub config: serde_json::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationConfig {
    pub financial_path: PathBuf,
    pub application_path: PathBuf,
    pub providers: Vec<ProcessConfig>,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub host_bounds: HostBounds,
    /// None loads persisted bounds (defaults for a fresh store). Changes require migration.
    #[serde(default)]
    pub conversation_limits: Option<crate::conversations::ConversationLimits>,
}
const MAX_CONFIG_BYTES: usize = 1024 * 1024;
fn invalid() -> AppError {
    AppError::new(
        ErrorKind::InvalidInput,
        "invalid application configuration",
        false,
    )
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let file = std::fs::File::open(path).map_err(|_| invalid())?;
    let mut bytes = Vec::new();
    file.take((MAX_CONFIG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(AppError::new(
            ErrorKind::ResourceLimit,
            "configuration exceeds size limit",
            false,
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}
impl ApplicationConfig {
    /// Resolve all relative paths against the configuration file, independent of caller cwd.
    pub async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        tokio::task::spawn_blocking(move || {
            let path = std::fs::canonicalize(path).map_err(|_| invalid())?;
            let base = path.parent().ok_or_else(invalid)?;
            let mut config: Self = read_json(&path)?;
            config.financial_path = base.join(config.financial_path);
            config.application_path = base.join(config.application_path);
            for provider in &mut config.providers {
                provider.manifest = base.join(&provider.manifest);
            }
            config.validate()?;
            Ok(config)
        })
        .await
        .map_err(|_| invalid())?
    }
    fn validate(&self) -> Result<()> {
        self.limits.validate()?;
        self.host_bounds.validate()?;
        if let Some(limits) = &self.conversation_limits {
            limits.validate()?;
        }
        crate::agent_contract::check_serialized_size(self, MAX_CONFIG_BYTES)?;
        if self.providers.len() > 1024
            || self.financial_path.as_os_str().is_empty()
            || self.application_path.as_os_str().is_empty()
            || self.financial_path == self.application_path
        {
            return Err(invalid());
        }
        Ok(())
    }
    /// Offline opening reads references without starting any configured child.
    pub async fn open(self, offline: bool) -> Result<Application> {
        self.validate()?;
        let (providers, repositories, store, limits, bounds, ids) =
            tokio::task::spawn_blocking(move || {
                let mut providers = Vec::new();
                let mut instance_ids = std::collections::BTreeSet::new();
                for config in &self.providers {
                    Scope {
                        workspace_id: config.instance_id.clone(),
                        request_id: "configuration".into(),
                        run_id: None,
                    }
                    .validate()?;
                    if !instance_ids.insert(&config.instance_id) {
                        return Err(AppError::new(
                            ErrorKind::Conflict,
                            "duplicate configured provider instance",
                            false,
                        ));
                    }
                }
                for config in self.providers.into_iter().filter(|_| !offline) {
                    let manifest: lugus_financial::plugin::Manifest = read_json(&config.manifest)?;
                    let directory = config.manifest.parent().ok_or_else(invalid)?.to_path_buf();
                    providers.push(ConfiguredProvider {
                        active: config.active && !offline,
                        factory: Arc::new(ProcessProviderFactory {
                            manifest,
                            directory,
                            instance_id: config.instance_id,
                            config: config.config,
                        }),
                    });
                }
                // Detect duplicates and invalid identities before creating databases.
                Catalog::new(
                    providers
                        .iter()
                        .map(|p| ProviderEntry {
                            identity: p.factory.identity(),
                            active: p.active,
                            available: false,
                            capabilities: Default::default(),
                        })
                        .collect(),
                )?;
                let ids = RandomIds::new()?;
                let store_ids = RandomIds::new()?;
                let repositories = Arc::new(SqliteRepositoryFactory::new(&self.financial_path));
                repositories.initialize().map_err(AppError::from)?;
                let evidence =
                    lugus_financial::storage::SqliteRepository::open(&self.financial_path)
                        .map_err(AppError::from)?;
                let store = if let Some(conversation_limits) = self.conversation_limits {
                    SqliteApplicationStore::open_with_conversation_limits(
                        &self.application_path,
                        Box::new(evidence),
                        self.limits.clone(),
                        conversation_limits,
                        Box::new(SystemClock),
                        Box::new(store_ids),
                    )?
                } else {
                    SqliteApplicationStore::open(
                        &self.application_path,
                        Box::new(evidence),
                        self.limits.clone(),
                        Box::new(SystemClock),
                        Box::new(store_ids),
                    )?
                };
                Ok::<_, AppError>((
                    providers,
                    repositories,
                    store,
                    self.limits,
                    self.host_bounds,
                    ids,
                ))
            })
            .await
            .map_err(|_| invalid())??;
        Application::start(
            providers,
            repositories,
            Box::new(store),
            limits,
            bounds,
            Box::new(ids),
        )
        .await
    }
}
