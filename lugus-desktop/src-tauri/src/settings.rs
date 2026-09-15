//! Native-only profiles and atomic persistence of the selected agent identity.
use crate::runtime::{AgentKind, Profile, RuntimeConfig, discover};
use lugus_app::{
    AppError, ErrorKind, Result,
    conversations::{ConversationOptions, RuntimeFactory},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Serialize)]
pub(crate) struct AgentOption {
    id: AgentKind,
    label: &'static str,
    available: bool,
    detail: String,
}
#[derive(Serialize)]
pub(crate) struct AgentSettings {
    pub selected: AgentKind,
    pub offline: bool,
    pub runtime_available: bool,
    agents: Vec<AgentOption>,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Preference {
    selected: AgentKind,
}
struct State {
    preference: Preference,
    profiles: BTreeMap<AgentKind, std::result::Result<Profile, String>>,
    path: PathBuf,
    offline: bool,
}
#[derive(Clone)]
pub(crate) struct Settings(Arc<Mutex<State>>);
fn failure(message: &str) -> AppError {
    AppError::new(ErrorKind::Unavailable, message, false)
}

impl Settings {
    pub fn open(
        config_path: &Path,
        legacy: Option<RuntimeConfig>,
        mut agents: BTreeMap<AgentKind, RuntimeConfig>,
        offline: bool,
    ) -> Result<Self> {
        let base = config_path
            .parent()
            .ok_or_else(|| failure("invalid desktop configuration directory"))?;
        let path = config_path.with_extension("agents.json");
        let saved = match std::fs::File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(4097)
                    .read_to_end(&mut bytes)
                    .map_err(|_| failure("Could not read saved agent selection"))?;
                if bytes.len() > 4096 {
                    return Err(failure("Saved agent selection exceeds size limit"));
                }
                Some(
                    serde_json::from_slice::<Preference>(&bytes)
                        .map_err(|_| failure("Invalid saved agent selection"))?,
                )
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(failure("Could not read saved agent selection")),
        };
        let mut profiles = BTreeMap::new();
        if !offline {
            let workspace = legacy
                .as_ref()
                .map(|c| base.join(&c.workspace))
                .or_else(|| agents.values().next().map(|c| base.join(&c.workspace)))
                .unwrap_or_else(|| base.join("runtime"));
            // Invalid workspace/bounds remain errors; an uninstalled CLI must not
            // prevent the user from opening Settings and selecting another agent.
            if let Some(config) = legacy.filter(|_| !agents.contains_key(&AgentKind::Codex)) {
                let profile = match Profile::resolve(AgentKind::Codex, config, base) {
                    Err(error) if error.kind == ErrorKind::InvalidInput => return Err(error),
                    result => result.map_err(|error| error.message),
                };
                profiles.insert(AgentKind::Codex, profile);
            }
            for kind in AgentKind::ALL {
                let explicit = agents.remove(&kind);
                let candidate = if explicit.is_some() {
                    explicit
                } else if profiles.contains_key(&kind) {
                    None
                } else if workspace.is_dir() {
                    discover(kind, &workspace)
                } else {
                    None
                };
                if let Some(config) = candidate {
                    profiles.insert(
                        kind,
                        Profile::resolve(kind, config, base).map_err(|e| e.message),
                    );
                }
            }
        }
        let preference = saved.unwrap_or_else(|| Preference {
            selected: if !profiles.get(&AgentKind::Codex).is_some_and(|p| p.is_ok())
                && profiles
                    .get(&AgentKind::ClaudeCode)
                    .is_some_and(|p| p.is_ok())
            {
                AgentKind::ClaudeCode
            } else {
                AgentKind::Codex
            },
        });
        Ok(Self(Arc::new(Mutex::new(State {
            preference,
            profiles,
            path,
            offline,
        }))))
    }
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn view(&self) -> AgentSettings {
        Self::view_state(&self.state())
    }
    fn view_state(state: &State) -> AgentSettings {
        let agents = AgentKind::ALL.into_iter().map(|id| {
            let (available, detail) = if state.offline {
                (false, "Agents are disabled while Lugus is in offline mode.".into())
            } else { match state.profiles.get(&id) {
                Some(Ok(_)) => (true, "Installed. Uses your existing CLI sign-in; connection is checked when a message starts.".into()),
                Some(Err(reason)) => (false, reason.clone()),
                None => (false, format!("Install and sign in to {}, then reopen Lugus. Custom installations can be configured in desktop.json.", id.label())),
            }};
            AgentOption { id, label: id.label(), available, detail }
        }).collect();
        AgentSettings {
            selected: state.preference.selected,
            offline: state.offline,
            runtime_available: !state.offline
                && state
                    .profiles
                    .get(&state.preference.selected)
                    .is_some_and(|p| p.is_ok()),
            agents,
        }
    }
    pub fn profile(&self) -> Result<Profile> {
        let state = self.state();
        if state.offline {
            return Err(failure("Agents are disabled in offline mode"));
        }
        state
            .profiles
            .get(&state.preference.selected)
            .and_then(|p| p.as_ref().ok())
            .cloned()
            .ok_or_else(|| failure("Select an installed agent in Settings to start chatting"))
    }

    pub fn options(&self) -> ConversationOptions {
        let mut options = ConversationOptions::default();
        options.run_limits.timeout = std::time::Duration::from_secs(
            self.state()
                .profiles
                .values()
                .filter_map(|p| p.as_ref().ok())
                .map(Profile::timeout_secs)
                .max()
                .unwrap_or(180),
        );
        options
    }
    pub async fn select(&self, agent: AgentKind) -> Result<AgentSettings> {
        let settings = self.clone();
        // The owned task commits disk and memory together even if the renderer drops its waiter.
        tokio::task::spawn_blocking(move || {
            let mut state = settings.state();
            if state.offline || !state.profiles.get(&agent).is_some_and(|p| p.is_ok()) {
                return Err(failure(
                    "This agent is unavailable. Check its installation and desktop configuration.",
                ));
            }
            let preference = Preference { selected: agent };
            let bytes = serde_json::to_vec_pretty(&preference)
                .map_err(|_| failure("Could not save agent selection"))?;
            let parent = state
                .path
                .parent()
                .ok_or_else(|| failure("Invalid settings directory"))?;
            let mut temporary = tempfile::NamedTempFile::new_in(parent)
                .map_err(|_| failure("Could not save agent selection"))?;
            temporary
                .write_all(&bytes)
                .and_then(|_| temporary.as_file().sync_all())
                .map_err(|_| failure("Could not save agent selection"))?;
            temporary
                .persist(&state.path)
                .map_err(|_| failure("Could not save agent selection"))?;
            state.preference = preference;
            Ok(Self::view_state(&state))
        })
        .await
        .map_err(|_| failure("Agent settings task stopped"))?
    }
}
#[async_trait::async_trait]
impl RuntimeFactory for Settings {
    fn snapshot(&self) -> Option<Arc<dyn RuntimeFactory>> {
        Some(match self.profile() {
            Ok(profile) => Arc::new(profile),
            Err(error) => Arc::new(Unavailable(error)),
        })
    }
    async fn create(&self) -> Result<Box<dyn lugus_agent::AgentRuntime>> {
        self.profile()?.create().await
    }
}
struct Unavailable(AppError);
#[async_trait::async_trait]
impl RuntimeFactory for Unavailable {
    async fn create(&self) -> Result<Box<dyn lugus_agent::AgentRuntime>> {
        Err(self.0.clone())
    }
}
