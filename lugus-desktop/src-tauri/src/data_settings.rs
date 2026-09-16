//! User-owned SEC contact settings; configuration writes preserve other providers.
use lugus_app::{AppError, Application, ApplicationConfig, ErrorKind, Operation, Result};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
const MAX_CONFIG: usize = 1024 * 1024;
#[derive(Clone)]
pub(crate) struct DataSettings(Arc<State>);
struct State {
    path: PathBuf,
    initial: Option<Value>,
    offline: bool,
    write: Mutex<()>,
}
#[derive(Serialize)]
pub(crate) struct View {
    revision: String,
    contact_name: String,
    contact_email: String,
    existing_identity: Option<String>,
    enabled: bool,
    installed: bool,
    ready: bool,
    restart_required: bool,
    offline: bool,
}
fn error(kind: ErrorKind, message: &str) -> AppError {
    AppError::new(kind, message, false)
}
fn storage() -> AppError {
    error(
        ErrorKind::Storage,
        "Could not read or save data-source settings",
    )
}
fn read(path: &Path) -> Result<(Vec<u8>, Value)> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| storage())?
        .take((MAX_CONFIG + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| storage())?;
    if bytes.len() > MAX_CONFIG {
        return Err(error(
            ErrorKind::ResourceLimit,
            "Application configuration exceeds the size limit",
        ));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| storage())?;
    serde_json::from_value::<ApplicationConfig>(value.clone()).map_err(|_| storage())?;
    Ok((bytes, value))
}
fn revision(bytes: &[u8]) -> String {
    let mut hash = DefaultHasher::new();
    bytes.hash(&mut hash);
    format!("{:016x}", hash.finish())
}
fn manifest(base: &Path, provider: &Value) -> Option<PathBuf> {
    provider["manifest"].as_str().map(|p| base.join(p))
}
fn sec_index(base: &Path, config: &Value) -> Result<Option<usize>> {
    let mut found = None;
    for (i, provider) in config["providers"]
        .as_array()
        .ok_or_else(storage)?
        .iter()
        .enumerate()
    {
        let is_sec = manifest(base, provider)
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .is_some_and(|m| m["id"] == "sec-edgar");
        if is_sec {
            if found.is_some() {
                return Err(error(
                    ErrorKind::Conflict,
                    "Multiple SEC providers are configured. Keep one SEC provider before editing its contact settings.",
                ));
            }
            found = Some(i);
        }
    }
    Ok(found)
}
fn discover(base: &Path, config: &Value) -> Option<PathBuf> {
    let mut candidates = vec![];
    for provider in config["providers"].as_array().into_iter().flatten() {
        if let Some(path) = manifest(base, provider).and_then(|p| {
            p.parent()?
                .parent()
                .map(|p| p.join("sec-edgar/plugin.json"))
        }) {
            candidates.push(path);
        }
    }
    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../lugus-financial/plugins/sec-edgar/plugin.json"),
    );
    candidates.into_iter().find(|p| p.is_file())
}
fn installed(path: &Path) -> bool {
    let Some(manifest) = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return false;
    };
    manifest["id"] == "sec-edgar"
        && manifest["command"].as_str().is_some_and(|command| {
            path.parent()
                .unwrap_or(Path::new("."))
                .join(command)
                .is_file()
        })
}
fn contact(name: &str, email: &str, enabled: bool) -> Result<()> {
    if name.len() > 160
        || email.len() > 254
        || !name.is_ascii()
        || !email.is_ascii()
        || name.chars().chain(email.chars()).any(char::is_control)
    {
        return Err(error(
            ErrorKind::InvalidInput,
            "Use plain ASCII contact details without line breaks (name up to 160 characters, email up to 254).",
        ));
    }
    if enabled || !name.is_empty() || !email.is_empty() {
        let valid_email = email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !domain.contains('@')
        }) && !email.chars().any(char::is_whitespace);
        if name.is_empty() || !valid_email {
            return Err(error(
                ErrorKind::InvalidInput,
                "Enter your contact name and a valid contact email for SEC research.",
            ));
        }
    }
    Ok(())
}
impl DataSettings {
    pub fn open(path: PathBuf, offline: bool) -> Result<Self> {
        let path = std::fs::canonicalize(path).map_err(|_| storage())?;
        let (_, config) = read(&path)?;
        // Ambiguous SEC configuration belongs in Settings, not app startup.
        let index = sec_index(path.parent().ok_or_else(storage)?, &config)
            .ok()
            .flatten();
        Ok(Self(Arc::new(State {
            path,
            initial: index.map(|i| config["providers"][i].clone()),
            offline,
            write: Mutex::new(()),
        })))
    }
    pub fn view(&self, app: &Application) -> Result<View> {
        let (bytes, config) = read(&self.0.path)?;
        let base = self.0.path.parent().ok_or_else(storage)?;
        let index = sec_index(base, &config)?;
        let current = index.map(|i| config["providers"][i].clone());
        let provider = current.as_ref().unwrap_or(&Value::Null);
        let path = manifest(base, provider).or_else(|| discover(base, &config));
        let restart_required = current != self.0.initial;
        let enabled = provider["active"].as_bool().unwrap_or(false);
        let id = provider["instance_id"].as_str().unwrap_or("");
        let offering = app.offering()?;
        let ready = enabled
            && !self.0.offline
            && !restart_required
            && offering.supports(id, Operation::Resolve)
            && offering.supports(id, Operation::Facts);
        Ok(View {
            revision: revision(&bytes),
            contact_name: provider["config"]["contact_name"]
                .as_str()
                .unwrap_or("")
                .into(),
            contact_email: provider["config"]["contact_email"]
                .as_str()
                .unwrap_or("")
                .into(),
            existing_identity: provider["config"]["user_agent"].as_str().map(str::to_owned),
            enabled,
            installed: path.as_deref().is_some_and(installed),
            ready,
            restart_required,
            offline: self.0.offline,
        })
    }
    pub async fn save(
        &self,
        app: Application,
        expected: String,
        name: String,
        email: String,
        enabled: bool,
    ) -> Result<View> {
        let settings = self.clone();
        tokio::task::spawn_blocking(move || {
            let _guard=settings.0.write.lock().map_err(|_|storage())?;
            let mut lock_path = settings.0.path.as_os_str().to_os_string();
            lock_path.push(".data-settings.lock");
            let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(PathBuf::from(lock_path)).map_err(|_| storage())?;
            lock.try_lock().map_err(|e| match e {
                std::fs::TryLockError::WouldBlock => error(ErrorKind::Conflict, "Data-source settings are being saved. Try again."),
                std::fs::TryLockError::Error(_) => storage(),
            })?;
            let name=name.trim();let email=email.trim();contact(name,email,enabled)?;
            let (bytes,mut config)=read(&settings.0.path)?;
            if revision(&bytes)!=expected{return Err(error(ErrorKind::Conflict,"Data-source settings changed. Close and reopen Settings before saving again."));}
            let base=settings.0.path.parent().ok_or_else(storage)?;
            let index=sec_index(base,&config)?;
            let path=index.and_then(|i|manifest(base,&config["providers"][i])).or_else(||discover(base,&config)).ok_or_else(||error(ErrorKind::Unavailable,"The SEC provider is not installed. Install the bundled SEC provider before enabling company research."))?;
            if enabled && !installed(&path) {return Err(error(ErrorKind::Unavailable,"The SEC provider's Python environment is missing. Install the bundled provider dependencies, then reopen Settings."));}
            let index=if let Some(index)=index {index} else {
                let providers=config["providers"].as_array_mut().ok_or_else(storage)?;
                if providers.iter().any(|p|p["instance_id"]=="sec-edgar") {return Err(error(ErrorKind::Conflict,"The SEC provider name is already used by another data source."));}
                let index=providers.len();
                providers.push(json!({"instance_id":"sec-edgar","manifest":path,"active":false,"config":{}}));index
            };
            let provider=&mut config["providers"][index];
            if !provider["config"].is_object() {provider["config"]=json!({});}
            provider["active"]=json!(enabled);
            provider["config"]["contact_name"]=json!(name);
            provider["config"]["contact_email"]=json!(email);
            if !name.is_empty() {provider["config"]["user_agent"]=json!(format!("Lugus {name} {email}"));}
            let output=serde_json::to_vec_pretty(&config).map_err(|_|storage())?;
            if output.len()>MAX_CONFIG{return Err(error(ErrorKind::ResourceLimit,"Application configuration exceeds the size limit"));}
            let mut temp=tempfile::NamedTempFile::new_in(base).map_err(|_|storage())?;
            temp.write_all(&output).and_then(|_|temp.as_file().sync_all()).map_err(|_|storage())?;
            if read(&settings.0.path)?.0 != bytes {
                return Err(error(ErrorKind::Conflict, "Data-source settings changed. Close and reopen Settings before saving again."));
            }
            temp.persist(&settings.0.path).map_err(|_|storage())?;
            settings.view(&app)
        }).await.map_err(|_|storage())?
    }
}
