//! Bounded native transport over the persisted application and conversation ports.
mod companies;
mod data_settings;
mod portfolio;
mod runtime;
mod settings;
use lugus_app::{conversations::*, *};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{io::Read, path::Path, sync::Arc};

const MAX_BYTES: usize = 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DesktopConfig {
    application_config: std::path::PathBuf,
    runtime: Option<runtime::RuntimeConfig>,
    #[serde(default)]
    agents: std::collections::BTreeMap<runtime::AgentKind, runtime::RuntimeConfig>,
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Company {
        command: companies::Command,
    },
    Portfolio {
        command: portfolio::Command,
    },
    Info,
    AgentSettings,
    DataSettings,
    WebSearchSettings,
    SaveWebSearchSettings {
        enabled: bool,
    },
    SaveDataSettings {
        revision: String,
        contact_name: String,
        contact_email: String,
        enabled: bool,
    },
    SelectAgent {
        agent: runtime::AgentKind,
    },
    Conversation {
        conversation: String,
    },
    List {
        #[serde(default)]
        offset: usize,
    },
    Create {
        request: String,
        title: String,
    },
    Messages {
        conversation: String,
        #[serde(default)]
        offset: usize,
    },
    Runs {
        conversation: String,
        #[serde(default)]
        offset: usize,
    },
    Send {
        #[serde(default)]
        company_hint: Option<String>,
        conversation: String,
        request: String,
        text: String,
        #[serde(default)]
        selected: Vec<SelectedReference>,
    },
    Status {
        conversation: String,
        run: String,
    },
    Activity {
        conversation: String,
        run: String,
        #[serde(default)]
        offset: usize,
    },
    Cancel {
        conversation: String,
        run: String,
    },
    Workspace {
        conversation: String,
    },
    View {
        conversation: String,
        view: String,
    },
    Read {
        conversation: String,
        view: String,
        #[serde(default)]
        offset: usize,
    },
    Binding {
        conversation: String,
        binding: String,
    },
    Select {
        conversation: String,
        view: String,
        revision: u64,
    },
    Presented {
        conversation: String,
        view: String,
        revision: u32,
        status: PresentationStatus,
    },
}
#[derive(Clone)]
pub struct Bridge {
    host: ConversationHost,
    runtime_available: bool,
    settings: Option<settings::Settings>,
    companies: Option<companies::Store>,
    data_settings: Option<data_settings::DataSettings>,
}
fn error(kind: ErrorKind, message: &'static str) -> AppError {
    AppError::new(kind, message, false)
}
fn value<T: Serialize>(item: T) -> Result<Value> {
    lugus_app::agent_contract::check_serialized_size(&item, isize::MAX as usize)?;
    serde_json::to_value(item)
        .map_err(|_| error(ErrorKind::InvalidInput, "response serialization failed"))
}
fn run_value(run: RunRecord) -> Value {
    json!({"id":run.id,"conversation_id":run.conversation_id,"workspace_id":run.workspace_id,"request_id":run.request_id,"user_message_id":run.user_message_id,"status":run.status,"created_at":run.created_at,"finished_at":run.finished_at,"error":run.error})
}
impl Bridge {
    pub fn from_host(host: ConversationHost, runtime_available: bool) -> Self {
        Self {
            host,
            runtime_available,
            settings: None,
            companies: None,
            data_settings: None,
        }
    }
    pub fn with_company_store(mut self, path: &Path) -> Result<Self> {
        self.companies = Some(companies::Store::open(path)?);
        Ok(self)
    }
    pub async fn open(path: &Path, offline: bool) -> Result<Self> {
        let path = path.to_owned();
        let (config, base, settings) = tokio::task::spawn_blocking(move || {
            let invalid = || error(ErrorKind::InvalidInput, "invalid desktop configuration");
            let path = std::fs::canonicalize(path).map_err(|_| invalid())?;
            let mut bytes = Vec::new();
            std::fs::File::open(&path)
                .map_err(|_| invalid())?
                .take((MAX_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| invalid())?;
            if bytes.len() > MAX_BYTES {
                return Err(error(
                    ErrorKind::ResourceLimit,
                    "desktop configuration exceeds size limit",
                ));
            }
            let config: DesktopConfig = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if config.application_config.as_os_str().is_empty() {
                return Err(invalid());
            }
            let settings = settings::Settings::open(
                &path,
                config.runtime.clone(),
                config.agents.clone(),
                offline,
            )?;
            Ok((
                config,
                path.parent().ok_or_else(invalid)?.to_owned(),
                settings,
            ))
        })
        .await
        .map_err(|_| error(ErrorKind::Unavailable, "configuration task stopped"))??;
        let available = settings.view().runtime_available;
        let application_path = base.join(config.application_config);
        let data_settings = data_settings::DataSettings::open(application_path.clone(), offline)?;
        let mut application = ApplicationConfig::load(application_path).await?;
        application.limits = lugus_app::Limits::unlimited_research();
        application.conversation_limits =
            Some(lugus_app::conversations::ConversationLimits::unlimited_research());
        let company_path = application
            .application_path
            .with_extension("companies.sqlite");
        let companies = tokio::task::spawn_blocking(move || companies::Store::open(&company_path))
            .await
            .map_err(|_| error(ErrorKind::Storage, "Company storage task stopped"))??;
        let app = application.open(offline).await?;
        let host =
            ConversationHost::start(app.clone(), Arc::new(settings.clone()), settings.options())
                .await;
        match host {
            Ok(host) => Ok(Self {
                host,
                runtime_available: available,
                settings: Some(settings),
                companies: Some(companies),
                data_settings: Some(data_settings),
            }),
            Err(err) => {
                let _ = app.shutdown().await;
                Err(err)
            }
        }
    }
    fn writable(&self) -> Result<()> {
        if self
            .settings
            .as_ref()
            .map_or(self.runtime_available, |s| s.view().runtime_available)
        {
            Ok(())
        } else {
            Err(error(
                ErrorKind::Unavailable,
                "Select an installed agent in Settings to start chatting; saved research is available offline.",
            ))
        }
    }
    fn page(&self, offset: usize) -> PageRequest {
        PageRequest {
            offset,
            limit: 100
                .min(self.host.limits().page_items)
                .min(self.host.application().limits().max_read_page_items),
        }
    }
    /// Read-only queries may exceed a byte budget even when their item count is valid.
    /// Reduce only the count; the store remains authoritative for continuation offsets.
    async fn bounded_page<T, F, Fut>(&self, offset: usize, read: F) -> Result<T>
    where
        T: Serialize,
        F: Fn(PageRequest) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut page = self.page(offset);
        loop {
            let result = read(page).await.and_then(|items| {
                lugus_app::agent_contract::check_serialized_size(
                    &items,
                    self.host.application().limits().max_output_bytes,
                )?;
                Ok(items)
            });
            match result {
                Err(err) if err.kind == ErrorKind::ResourceLimit && page.limit > 1 => {
                    page.limit = (page.limit / 2).max(1);
                }
                result => return result,
            }
        }
    }
    async fn scope(&self, conversation: &str) -> Result<Scope> {
        let c = self.host.conversation(conversation).await?;
        self.host
            .application()
            .scope(&c.workspace_id, "desktop-read", None)
    }
    pub async fn dispatch(&self, payload: &str) -> Result<Value> {
        if payload.len() > self.host.application().limits().max_input_bytes {
            return Err(error(
                ErrorKind::ResourceLimit,
                "desktop command exceeds size limit",
            ));
        }
        let command: Command = serde_json::from_str(payload)
            .map_err(|_| error(ErrorKind::InvalidInput, "invalid desktop command"))?;
        let app = self.host.application();
        let response = match command {
            Command::Company { command } => companies::dispatch(self, command).await?,
            Command::Portfolio { command } => portfolio::dispatch(app, command).await?,
            Command::Info => match &self.settings {
                Some(settings) => {
                    let view = settings.view();
                    json!({"runtime_available":view.runtime_available,"offline":view.offline,"agent":view.selected})
                }
                None => {
                    json!({"runtime_available":self.runtime_available,"offline":!self.runtime_available})
                }
            },
            Command::WebSearchSettings => value(
                self.settings
                    .as_ref()
                    .ok_or_else(|| {
                        error(
                            ErrorKind::Unavailable,
                            "Internet search settings are unavailable for this host",
                        )
                    })?
                    .web_search_view(),
            )?,
            Command::SaveWebSearchSettings { enabled } => value(
                self.settings
                    .as_ref()
                    .ok_or_else(|| {
                        error(
                            ErrorKind::Unavailable,
                            "Internet search settings are unavailable for this host",
                        )
                    })?
                    .save_web_search(enabled)
                    .await?,
            )?,
            Command::DataSettings => value(
                self.data_settings
                    .as_ref()
                    .ok_or_else(|| {
                        error(
                            ErrorKind::Unavailable,
                            "Data-source settings are unavailable for this host",
                        )
                    })?
                    .view(app)?,
            )?,
            Command::SaveDataSettings {
                revision,
                contact_name,
                contact_email,
                enabled,
            } => value(
                self.data_settings
                    .as_ref()
                    .ok_or_else(|| {
                        error(
                            ErrorKind::Unavailable,
                            "Data-source settings are unavailable for this host",
                        )
                    })?
                    .save(app.clone(), revision, contact_name, contact_email, enabled)
                    .await?,
            )?,
            Command::AgentSettings => value(
                self.settings
                    .as_ref()
                    .ok_or_else(|| {
                        error(
                            ErrorKind::Unavailable,
                            "Agent settings are not available for this host",
                        )
                    })?
                    .view(),
            )?,
            Command::SelectAgent { agent } => value(
                self.settings
                    .as_ref()
                    .ok_or_else(|| {
                        error(
                            ErrorKind::Unavailable,
                            "Agent settings are not available for this host",
                        )
                    })?
                    .select(agent)
                    .await?,
            )?,
            Command::List { offset } => value(
                self.bounded_page(offset, |page| app.recent_conversations(page))
                    .await?,
            )?,
            Command::Conversation { conversation } => {
                value(self.host.conversation(&conversation).await?)?
            }
            Command::Create { request, title } => value(self.host.create(&request, &title).await?)?,
            Command::Messages {
                conversation,
                offset,
            } => value(
                self.bounded_page(offset, |page| self.host.messages(&conversation, page))
                    .await?,
            )?,
            Command::Runs {
                conversation,
                offset,
            } => {
                let page = self
                    .bounded_page(offset, |page| self.host.runs(&conversation, page))
                    .await?;
                json!({"items":page.items.into_iter().map(run_value).collect::<Vec<_>>(),"next_offset":page.next_offset})
            }
            Command::Send {
                company_hint,
                conversation,
                request,
                text,
                selected,
            } => {
                self.writable()?;
                // Company context is a host invariant, including clients that open a
                // company conversation from the ordinary recent-conversation list.
                if let Some(store) = &self.companies {
                    match store.get(&conversation) {
                        Ok(company) => {
                            return companies::dispatch(
                                self,
                                companies::Command::Send {
                                    company: company.id,
                                    request,
                                    text,
                                    selected,
                                    review: false,
                                },
                            )
                            .await;
                        }
                        Err(error) if error.kind == ErrorKind::MissingData => {}
                        Err(error) => return Err(error),
                    }
                }
                run_value(
                    self.host
                        .send(SendMessageRequest {
                            research_brief: None,
                            company_hint,
                            conversation_id: conversation,
                            request_id: request,
                            text,
                            selected,
                        })
                        .await?,
                )
            }
            Command::Status { conversation, run } => {
                run_value(self.host.status(&conversation, &run).await?)
            }
            Command::Activity {
                conversation,
                run,
                offset,
            } => value(
                self.bounded_page(offset, |page| self.host.activity(&conversation, &run, page))
                    .await?,
            )?,
            Command::Cancel { conversation, run } => {
                run_value(self.host.cancel(&conversation, &run).await?)
            }
            Command::Workspace { conversation } => {
                value(self.host.workspace(&conversation).await?)?
            }
            Command::View { conversation, view } => value(
                app.read_view(&self.scope(&conversation).await?, &view)
                    .await?,
            )?,
            Command::Read {
                conversation,
                view,
                offset,
            } => {
                let scope = self.scope(&conversation).await?;
                let view = app.read_view(&scope, &view).await?;
                value(
                    self.bounded_page(offset, |page| {
                        app.read_dataset(&scope, &view.dataset_id, page)
                    })
                    .await?,
                )?
            }
            Command::Binding {
                conversation,
                binding,
            } => value(
                app.read_binding(&self.scope(&conversation).await?, &binding)
                    .await?,
            )?,
            Command::Select {
                conversation,
                view,
                revision,
            } => value(
                self.host
                    .mutate_workspace(
                        &conversation,
                        revision,
                        WorkspaceMutation::Select { view_id: view },
                    )
                    .await?,
            )?,
            Command::Presented {
                conversation,
                view,
                revision,
                status,
            } => value(
                app.report_presentation(
                    &self.scope(&conversation).await?,
                    PresentationResult {
                        view_id: view,
                        descriptor_revision: revision,
                        status,
                    },
                )
                .await?,
            )?,
        };
        lugus_app::agent_contract::check_serialized_size(&response, app.limits().max_output_bytes)?;
        Ok(response)
    }
    pub async fn shutdown(&self) -> Result<()> {
        self.host.shutdown().await
    }
}

#[cfg(test)]
mod research_budget_tests {
    use super::*;
    #[tokio::test]
    async fn desktop_removes_persisted_research_budgets() {
        let dir = tempfile::tempdir().unwrap();
        let application = dir.path().join("application.json");
        std::fs::write(&application, serde_json::json!({"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[]}).to_string()).unwrap();
        let old = ApplicationConfig::load(&application)
            .await
            .unwrap()
            .open(true)
            .await
            .unwrap();
        old.shutdown().await.unwrap();
        drop(old);
        let desktop = dir.path().join("desktop.json");
        std::fs::write(&desktop, r#"{"application_config":"application.json"}"#).unwrap();
        let bridge = Bridge::open(&desktop, true).await.unwrap();
        assert_eq!(bridge.host.limits().activity_events, isize::MAX as usize);
        assert_eq!(bridge.host.limits().assistant_bytes, isize::MAX as usize);
        assert_eq!(
            bridge.host.application().limits().operation_timeout,
            std::time::Duration::MAX
        );
        bridge.shutdown().await.unwrap();
    }
}
