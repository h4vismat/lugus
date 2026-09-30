//! Investor-owned company records; agent turns receive immutable brief snapshots.
use crate::{Bridge, error, value};
use lugus_app::{conversations::*, *};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct Store(Arc<Mutex<Connection>>);
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Company {
    pub id: String,
    pub name: String,
    pub hint: String,
    pub conversation_id: String,
    pub revision: u64,
    pub thesis: String,
    pub questions: String,
    pub findings: Vec<Finding>,
    pub updated_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    pub message_id: String,
    pub run_id: String,
    pub text: String,
    pub evidence: Vec<SelectedReference>,
    pub created_at: String,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    List {
        #[serde(default)]
        offset: usize,
    },
    Create {
        request: String,
        name: String,
        hint: String,
    },
    Get {
        company: String,
    },
    Save {
        company: String,
        revision: u64,
        thesis: String,
        questions: String,
    },
    Finding {
        company: String,
        revision: u64,
        message: String,
        text: String,
    },
    History {
        company: String,
        #[serde(default)]
        offset: usize,
    },
    Send {
        company: String,
        request: String,
        text: String,
        #[serde(default)]
        selected: Vec<SelectedReference>,
        #[serde(default)]
        review: bool,
    },
}
fn storage(_: impl std::fmt::Debug) -> AppError {
    error(
        ErrorKind::Storage,
        "Company research could not be stored or read",
    )
}
fn missing() -> AppError {
    error(ErrorKind::MissingData, "Company research was not found")
}
fn bounded(text: &str, max: usize, required: bool) -> Result<()> {
    if text.len() > max
        || (required && text.trim().is_empty())
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        Err(error(
            ErrorKind::InvalidInput,
            "Company text is empty or exceeds its size limit",
        ))
    } else {
        Ok(())
    }
}
fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path).map_err(storage)?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(storage)?;
        db.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS companies (id TEXT PRIMARY KEY, creation TEXT NOT NULL UNIQUE, payload TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS company_revisions (company TEXT NOT NULL, revision INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(company,revision));
            CREATE TABLE IF NOT EXISTS company_sends (company TEXT NOT NULL, request TEXT NOT NULL, original TEXT NOT NULL, frozen TEXT NOT NULL, review INTEGER NOT NULL, run TEXT, created_at TEXT NOT NULL, PRIMARY KEY(company,request));").map_err(storage)?;
        Ok(Self(Arc::new(Mutex::new(db))))
    }
    fn get_from(db: &Connection, id: &str) -> Result<Company> {
        let raw: String = db
            .query_row("SELECT payload FROM companies WHERE id=?", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(storage)?
            .ok_or_else(missing)?;
        serde_json::from_str(&raw).map_err(storage)
    }
    pub fn get(&self, id: &str) -> Result<Company> {
        Self::get_from(&*self.0.lock().map_err(storage)?, id)
    }
    fn list(&self, offset: usize) -> Result<Value> {
        let db = self.0.lock().map_err(storage)?;
        let mut query = db
            .prepare("SELECT payload FROM companies ORDER BY rowid LIMIT 101 OFFSET ?")
            .map_err(storage)?;
        let rows = query
            .query_map([i64::try_from(offset).map_err(storage)?], |r| {
                r.get::<_, String>(0)
            })
            .map_err(storage)?;
        let mut items = Vec::new();
        for raw in rows {
            let company: Company = serde_json::from_str(&raw.map_err(storage)?).map_err(storage)?;
            items.push(json!({"id":company.id,"name":company.name,"hint":company.hint,"conversation_id":company.conversation_id,"revision":company.revision,"updated_at":company.updated_at}));
        }
        let next = (items.len() > 100).then_some(offset + 100);
        items.truncate(100);
        Ok(json!({"items":items,"next_offset":next}))
    }
    fn create(&self, creation: &str, company: Company) -> Result<Company> {
        let mut db = self.0.lock().map_err(storage)?;
        let tx = db.transaction().map_err(storage)?;
        if let Some(raw) = tx
            .query_row(
                "SELECT payload FROM companies WHERE creation=?",
                [creation],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?
        {
            let previous: Company = serde_json::from_str(&raw).map_err(storage)?;
            if previous.name != company.name || previous.hint != company.hint {
                return Err(error(
                    ErrorKind::Conflict,
                    "Company creation request was already used",
                ));
            }
            return Ok(previous);
        }
        lugus_app::agent_contract::check_serialized_size(&company, isize::MAX as usize)?;
        let raw = serde_json::to_string(&company).map_err(storage)?;
        tx.execute(
            "INSERT INTO companies VALUES (?,?,?)",
            params![company.id, creation, raw],
        )
        .map_err(storage)?;
        tx.execute(
            "INSERT INTO company_revisions VALUES (?,0,?)",
            params![company.id, raw],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(company)
    }
    fn update(
        &self,
        id: &str,
        revision: u64,
        change: impl FnOnce(&mut Company) -> Result<()>,
    ) -> Result<Company> {
        let mut db = self.0.lock().map_err(storage)?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut company = Self::get_from(&tx, id)?;
        if company.revision != revision {
            return Err(error(
                ErrorKind::Conflict,
                "This company changed. Reload its saved brief before saving again; keep a copy of your edits.",
            ));
        }
        change(&mut company)?;
        company.revision += 1;
        company.updated_at = timestamp();
        lugus_app::agent_contract::check_serialized_size(&company, isize::MAX as usize)?;
        let raw = serde_json::to_string(&company).map_err(storage)?;
        tx.execute(
            "UPDATE companies SET payload=? WHERE id=?",
            params![raw, id],
        )
        .map_err(storage)?;
        tx.execute(
            "INSERT INTO company_revisions VALUES (?,?,?)",
            params![id, i64::try_from(company.revision).map_err(storage)?, raw],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(company)
    }
    fn history(&self, id: &str, offset: usize) -> Result<Value> {
        self.get(id)?;
        let db = self.0.lock().map_err(storage)?;
        let mut query = db.prepare("SELECT payload FROM company_revisions WHERE company=? ORDER BY revision DESC LIMIT 6 OFFSET ?").map_err(storage)?;
        let mut revisions = Vec::new();
        for row in query
            .query_map(params![id, i64::try_from(offset).map_err(storage)?], |r| {
                r.get::<_, String>(0)
            })
            .map_err(storage)?
        {
            let company: Company = serde_json::from_str(&row.map_err(storage)?).map_err(storage)?;
            revisions.push(json!({"revision":company.revision,"thesis":company.thesis,"questions":company.questions,"updated_at":company.updated_at}));
        }
        let next = (revisions.len() > 5).then_some(offset + 5);
        revisions.truncate(5);
        let mut query = db.prepare("SELECT run,created_at,frozen FROM company_sends WHERE company=? AND review=1 ORDER BY rowid DESC LIMIT 6 OFFSET ?").map_err(storage)?;
        let mut reviews = Vec::new();
        for row in query
            .query_map(params![id, i64::try_from(offset).map_err(storage)?], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(storage)?
        {
            let (run, date, frozen) = row.map_err(storage)?;
            let frozen: SendMessageRequest = serde_json::from_str(&frozen).map_err(storage)?;
            reviews.push(json!({"run_id":run,"created_at":date,"brief":frozen.research_brief}));
        }
        let more_reviews = reviews.len() > 5;
        reviews.truncate(5);
        Ok(
            json!({"revisions":revisions,"reviews":reviews,"next_offset":if more_reviews {Some(offset+5)} else {next}}),
        )
    }
    fn freeze(
        &self,
        company: &Company,
        request: &str,
        text: &str,
        selected: Vec<SelectedReference>,
        review: bool,
        previous_review: Option<Value>,
    ) -> Result<SendMessageRequest> {
        validate_id(request)?;
        let original = json!({"text":text,"selected":selected,"review":review}).to_string();
        let mut db = self.0.lock().map_err(storage)?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        if let Some((prior, frozen)) = tx
            .query_row(
                "SELECT original,frozen FROM company_sends WHERE company=? AND request=?",
                params![company.id, request],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(storage)?
        {
            if prior != original {
                return Err(error(
                    ErrorKind::Conflict,
                    "Company message request was already used with different content",
                ));
            }
            return serde_json::from_str(&frozen).map_err(storage);
        }
        // Investor context has no research-size budget; source evidence remains in scoped tools.
        let findings: Vec<_> = company.findings.iter().rev().map(|f| json!({"text":f.text,"message_id":f.message_id,"run_id":f.run_id,"saved_at":f.created_at,"evidence":f.evidence})).collect();
        let brief = json!({"company":company.name,"resolution_hint":company.hint,"identity_status":"user_entered_unverified","revision":company.revision,"updated_at":company.updated_at,"thesis":company.thesis,"open_questions":company.questions,"accepted_findings":findings,"omitted_findings":0,"previous_review":previous_review,"review_requested":review});
        let brief = brief.to_string();
        let frozen = SendMessageRequest {
            research_brief: Some(brief),
            company_hint: Some(company.hint.clone()),
            conversation_id: company.conversation_id.clone(),
            request_id: format!("company:{request}"),
            text: text.into(),
            selected,
        };
        frozen.validate(&ConversationLimits::unlimited_research())?;
        let raw = serde_json::to_string(&frozen).map_err(storage)?;
        tx.execute(
            "INSERT INTO company_sends VALUES (?,?,?,?,?,NULL,?)",
            params![company.id, request, original, raw, review, timestamp()],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(frozen)
    }
    fn record_run(&self, company: &str, request: &str, run: &str) -> Result<()> {
        self.0
            .lock()
            .map_err(storage)?
            .execute(
                "UPDATE company_sends SET run=? WHERE company=? AND request=?",
                params![run, company, request],
            )
            .map_err(storage)?;
        Ok(())
    }
}

async fn reconcile(bridge: &Bridge, store: &Store, company: &Company) -> Result<()> {
    let unresolved: Vec<(String, SendMessageRequest)> = {
        let db = store.0.lock().map_err(storage)?;
        let mut query = db.prepare("SELECT request,frozen FROM company_sends WHERE company=? AND run IS NULL LIMIT 1000").map_err(storage)?;
        let rows = query
            .query_map([&company.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(storage)?;
        let mut unresolved = Vec::new();
        for row in rows {
            let (request, frozen) = row.map_err(storage)?;
            unresolved.push((request, serde_json::from_str(&frozen).map_err(storage)?));
        }
        unresolved
    };
    if unresolved.is_empty() {
        return Ok(());
    }
    let mut offset = 0;
    loop {
        let page = bridge
            .host
            .runs(&company.conversation_id, bridge.page(offset))
            .await?;
        for run in page.items {
            if let Some((request, _)) = unresolved
                .iter()
                .find(|(_, frozen)| frozen.request_id == run.request_id)
            {
                store.record_run(&company.id, request, &run.id)?;
            }
        }
        match page.next_offset {
            Some(next) if next > offset => offset = next,
            None => return Ok(()),
            _ => return Err(storage("pagination")),
        }
    }
}

async fn messages(bridge: &Bridge, conversation: &str) -> Result<Vec<Message>> {
    let mut offset = 0;
    let mut result = Vec::new();
    loop {
        let page = bridge
            .host
            .messages(conversation, bridge.page(offset))
            .await?;
        result.extend(page.items);
        match page.next_offset {
            Some(next) if next > offset => offset = next,
            None => return Ok(result),
            _ => return Err(storage("pagination")),
        }
    }
}
async fn previous_review(
    bridge: &Bridge,
    company: &Company,
    store: &Store,
) -> Result<Option<Value>> {
    let messages = messages(bridge, &company.conversation_id).await?;
    let mut history = store.history(&company.id, 0)?;
    loop {
        for review in history["reviews"].as_array().into_iter().flatten() {
            if let Some(run) = review["run_id"].as_str() {
                let status = bridge.host.status(&company.conversation_id, run).await?;
                if status.status == RunStatus::Completed
                    && let Some(message) = messages
                        .iter()
                        .find(|m| m.run_id == run && m.role == MessageRole::Assistant)
                {
                    let excerpt = message.text.clone();
                    return Ok(Some(
                        json!({"run_id":run,"created_at":message.created_at,"excerpt":excerpt,"truncated":false}),
                    ));
                }
            }
        }
        let Some(offset) = history["next_offset"].as_u64() else {
            return Ok(None);
        };
        history = store.history(&company.id, usize::try_from(offset).map_err(storage)?)?;
    }
}
pub async fn dispatch(bridge: &Bridge, command: Command) -> Result<Value> {
    let store = bridge.companies.as_ref().ok_or_else(|| {
        error(
            ErrorKind::Unavailable,
            "Company storage is unavailable for this host",
        )
    })?;
    match command {
        Command::List { offset } => store.list(offset),
        Command::Create {
            request,
            name,
            hint,
        } => {
            validate_id(&request)?;
            bounded(&name, 200, true)?;
            bounded(&hint, 200, true)?;
            if name.chars().any(char::is_control) || hint.chars().any(char::is_control) {
                return Err(error(
                    ErrorKind::InvalidInput,
                    "Company name and hint must be control-free",
                ));
            }
            let conversation = bridge
                .host
                .create(&format!("company:{request}"), &name)
                .await?;
            value(store.create(
                &request,
                Company {
                    id: conversation.id.clone(),
                    conversation_id: conversation.id,
                    name,
                    hint,
                    revision: 0,
                    thesis: String::new(),
                    questions: String::new(),
                    findings: vec![],
                    updated_at: timestamp(),
                },
            )?)
        }
        Command::Get { company } => value(store.get(&company)?),
        Command::Save {
            company,
            revision,
            thesis,
            questions,
        } => {
            bounded(&thesis, isize::MAX as usize, false)?;
            bounded(&questions, isize::MAX as usize, false)?;
            value(store.update(&company, revision, |c| {
                c.thesis = thesis;
                c.questions = questions;
                Ok(())
            })?)
        }
        Command::Finding {
            company,
            revision,
            message,
            text,
        } => {
            bounded(&text, isize::MAX as usize, true)?;
            let current = store.get(&company)?;
            let source = messages(bridge, &current.conversation_id)
                .await?
                .into_iter()
                .find(|m| m.id == message && m.role == MessageRole::Assistant)
                .ok_or_else(missing)?;
            let run = bridge
                .host
                .status(&current.conversation_id, &source.run_id)
                .await?;
            if run.status != RunStatus::Completed {
                return Err(error(
                    ErrorKind::Conflict,
                    "Only completed answers can be saved as findings",
                ));
            }
            let mut evidence: Vec<_> = run
                .input
                .references
                .iter()
                .map(|r| r.reference.clone())
                .collect();
            if let Some(raw) = bridge
                .host
                .application()
                .conversation_preparation(&current.conversation_id, &source.run_id)
                .await?
            {
                let prepared: lugus_app::research::PreparedResearch =
                    serde_json::from_str(&raw).map_err(storage)?;
                evidence.extend(
                    prepared
                        .views
                        .into_iter()
                        .map(|v| SelectedReference::View { id: v.id }),
                );
            }
            let mut seen = std::collections::HashSet::new();
            evidence.retain(|r| seen.insert(r.clone()));
            value(store.update(&company, revision, |c| {
                c.findings.push(Finding {
                    message_id: source.id,
                    run_id: source.run_id,
                    text,
                    evidence,
                    created_at: timestamp(),
                });
                Ok(())
            })?)
        }
        Command::History { company, offset } => {
            let c = store.get(&company)?;
            reconcile(bridge, store, &c).await?;
            let mut history = store.history(&company, offset)?;
            let messages = messages(bridge, &c.conversation_id).await?;
            for review in history["reviews"].as_array_mut().unwrap() {
                if let Some(run) = review["run_id"].as_str().map(str::to_owned) {
                    let status = bridge.host.status(&c.conversation_id, &run).await?;
                    review["status"] = json!(status.status);
                    review["error"] = json!(status.error);
                    review["text"] = messages
                        .iter()
                        .find(|m| m.run_id == run && m.role == MessageRole::Assistant)
                        .map(|m| json!(m.text))
                        .unwrap_or(Value::Null);
                } else {
                    review["status"] = json!("not_started");
                }
            }
            Ok(history)
        }
        Command::Send {
            company,
            request,
            text,
            selected,
            review,
        } => {
            bridge.writable()?;
            let c = store.get(&company)?;
            reconcile(bridge, store, &c).await?;
            let previous = previous_review(bridge, &c, store).await?;
            let frozen = store.freeze(&c, &request, &text, selected, review, previous)?;
            let run = bridge.host.send(frozen).await?;
            store.record_run(&company, &request, &run.id)?;
            Ok(crate::run_value(run))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_briefs_preserve_all_investor_context() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("companies.sqlite")).unwrap();
        let company = Company {
            id: "c".into(),
            name: "Company".into(),
            hint: "C".into(),
            conversation_id: "c".into(),
            revision: 0,
            thesis: "x".repeat(6000),
            questions: "q".repeat(3000),
            updated_at: timestamp(),
            findings: (0..6)
                .map(|_| Finding {
                    message_id: "message".into(),
                    run_id: "run".into(),
                    text: "f".repeat(1500),
                    evidence: vec![],
                    created_at: timestamp(),
                })
                .collect(),
        };
        let frozen = store
            .freeze(
                &company,
                "large",
                "question",
                vec![],
                false,
                Some(json!({"excerpt":"界".repeat(2000)})),
            )
            .unwrap();
        let raw = frozen.research_brief.unwrap();
        assert!(raw.len() > 24 * 1024);
        let brief: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(brief["thesis"], company.thesis);
        assert_eq!(brief["open_questions"], company.questions);
        assert_eq!(brief["omitted_findings"], 0);
        assert_eq!(brief["accepted_findings"].as_array().unwrap().len(), 6);
        assert_eq!(brief["previous_review"]["excerpt"], "界".repeat(2000));
    }
}
