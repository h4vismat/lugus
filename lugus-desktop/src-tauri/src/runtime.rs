//! Immutable runtime profiles. Renderer commands select identities, never executables.
use lugus_agent::{
    AgentRuntime,
    claude::{ClaudeConfig, ClaudeRuntime},
    codex::{CodexConfig, CodexRuntime},
};
use lugus_app::{AppError, ErrorKind, Result, conversations::RuntimeFactory};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentKind {
    #[default]
    Codex,
    ClaudeCode,
}
impl AgentKind {
    pub const ALL: [Self; 2] = [Self::Codex, Self::ClaudeCode];
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
        }
    }
    fn command(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude",
        }
    }
    fn environment(self) -> &'static str {
        match self {
            Self::Codex => "LUGUS_CODEX",
            Self::ClaudeCode => "LUGUS_CLAUDE",
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeConfig {
    pub executable: PathBuf,
    pub workspace: PathBuf,
    pub model: Option<String>,
    pub model_provider: Option<String>,
    #[serde(default, rename = "timeout_secs")]
    pub _timeout_secs: u64,
}
#[derive(Clone)]
pub(crate) struct Profile {
    pub kind: AgentKind,
    pub allow_web_search: bool,
    config: RuntimeConfig,
}
impl Profile {
    pub fn resolve(kind: AgentKind, mut config: RuntimeConfig, base: &Path) -> Result<Self> {
        let invalid = || {
            AppError::new(
                ErrorKind::InvalidInput,
                "runtime needs an executable and an explicit existing non-repository working directory",
                false,
            )
        };
        if config.executable.as_os_str().is_empty() || config.workspace.as_os_str().is_empty() {
            return Err(invalid());
        }
        config.workspace =
            std::fs::canonicalize(base.join(&config.workspace)).map_err(|_| invalid())?;
        if !config.workspace.is_dir()
            || config
                .workspace
                .ancestors()
                .any(|p| p.join(".git").exists())
        {
            return Err(invalid());
        }
        let unavailable = || {
            AppError::new(
                ErrorKind::Unavailable,
                format!(
                    "{} executable is missing. Install it or update desktop.json, then reopen Lugus.",
                    kind.label()
                ),
                false,
            )
        };
        config.executable =
            std::fs::canonicalize(base.join(&config.executable)).map_err(|_| unavailable())?;
        if !config.executable.is_file() {
            return Err(unavailable());
        }
        if kind == AgentKind::ClaudeCode && config.model_provider.is_some() {
            return Err(AppError::new(
                ErrorKind::InvalidInput,
                "Claude Code uses its own provider configuration; omit model_provider",
                false,
            ));
        }
        Ok(Self {
            kind,
            config,
            allow_web_search: false,
        })
    }
}
#[async_trait::async_trait]
impl RuntimeFactory for Profile {
    fn web_search(&self) -> Option<bool> {
        Some(self.allow_web_search)
    }
    fn run_timeout(&self) -> Option<std::time::Duration> {
        None
    }
    async fn create(&self) -> Result<Box<dyn AgentRuntime>> {
        let config = &self.config;
        let runtime: lugus_agent::Result<Box<dyn AgentRuntime>> = match self.kind {
            AgentKind::Codex => CodexRuntime::connect(CodexConfig {
                executable: config.executable.clone(),
                workspace: config.workspace.clone(),
                model: config.model.clone(),
                model_provider: config.model_provider.clone(),
            })
            .await
            .map(|r| Box::new(r) as Box<dyn AgentRuntime>),
            AgentKind::ClaudeCode => ClaudeRuntime::connect(ClaudeConfig {
                executable: config.executable.clone(),
                workspace: config.workspace.clone(),
                model: config.model.clone(),
            })
            .await
            .map(|r| Box::new(r) as Box<dyn AgentRuntime>),
        };
        let runtime = runtime.map_err(|e| {
            AppError::new(
                match &e {
                    lugus_agent::Error::Configuration(_) => ErrorKind::Unsupported,
                    lugus_agent::Error::AuthenticationRequired => ErrorKind::AuthenticationRequired,
                    lugus_agent::Error::NeedsAttention(_) => ErrorKind::NeedsAttention,
                    _ => ErrorKind::Unavailable,
                },
                format!("{} unavailable: {e}", self.kind.label()),
                false,
            )
        })?;
        Ok(runtime)
    }
}
/// Discovery never runs executables or reads CLI account configuration.
pub(crate) fn discover(kind: AgentKind, workspace: &Path) -> Option<RuntimeConfig> {
    let explicit = std::env::var_os(kind.environment()).map(PathBuf::from);
    let executable = if let Some(path) = explicit {
        path
    } else {
        let mut candidates = Vec::new();
        if let Some(home) = std::env::var_os("HOME") {
            candidates.push(PathBuf::from(home).join(".local/bin").join(kind.command()));
        }
        if let Some(path) = std::env::var_os("PATH") {
            candidates.extend(std::env::split_paths(&path).map(|p| p.join(kind.command())));
        }
        candidates.extend(
            ["/opt/homebrew/bin", "/usr/local/bin"].map(|p| Path::new(p).join(kind.command())),
        );
        candidates.into_iter().find(|p| p.is_file())?
    };
    Some(RuntimeConfig {
        executable,
        workspace: workspace.to_owned(),
        model: None,
        model_provider: None,
        _timeout_secs: 0,
    })
}
