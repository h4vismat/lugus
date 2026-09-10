//! SQLite evidence history. Each page and its continuation cursor commit together.
pub mod selection;
pub mod bounded;
use crate::{
    domain::*,
    error::{Error, ErrorKind, Result},
};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Complete,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: i64,
    pub provider: ProviderIdentity,
    pub query: Query,
    pub operation: String,
    pub status: RunStatus,
    pub filings_cursor: Option<String>,
    pub facts_cursor: Option<String>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactObservation {
    pub observation_id: i64,
    pub fact: Fact,
    pub metric: Option<Metric>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub runs: Vec<Run>,
    pub filings: Vec<Filing>,
    pub facts: Vec<FactObservation>,
}
impl Snapshot {
    /// This reports run completion only, not source completeness or range coverage.
    pub fn is_complete(&self) -> bool {
        !self.runs.is_empty() && self.runs.iter().all(|r| r.status == RunStatus::Complete)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentObservation {
    pub provider: ProviderIdentity,
    pub checksum: String,
    pub source_url: String,
    pub media_type: String,
    pub retrieved_at: chrono::DateTime<Utc>,
}

pub trait Repository {
    fn start_run(
        &mut self,
        provider: &ProviderIdentity,
        query: &Query,
        operation: &str,
    ) -> Result<i64>;
    fn save_filings_page(&mut self, run: i64, page: &Page<Filing>) -> Result<()>;
    fn save_facts_page(
        &mut self,
        run: i64,
        page: &Page<Fact>,
        mapper: &dyn Fn(&Fact) -> Option<Metric>,
    ) -> Result<()>;
    fn finish_run(&mut self, run: i64, error: Option<&Error>) -> Result<()>;
    fn snapshot(&self, provider: &ProviderIdentity, query: &Query) -> Result<Snapshot>;
    fn save_document(
        &mut self,
        provider: &ProviderIdentity,
        document: &Document,
        max_bytes: usize,
    ) -> Result<String>;
    fn stored_document(&self, checksum: &str) -> Result<Vec<u8>>;
}
/// Links immutable observation revisions to each run's actual retrieval time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationRetrieval {
    pub observation_id: i64,
    pub kind: String,
    pub fingerprint: String,
    pub retrieved_at: chrono::DateTime<Utc>,
}
pub struct SqliteRepository {
    pub(crate) connection: Connection,
}
fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidRequest, message)
}
fn validate_identity(p: &ProviderIdentity) -> Result<()> {
    if [&p.instance_id, &p.plugin_id, &p.plugin_version]
        .iter()
        .any(|s| s.trim().is_empty())
    {
        Err(invalid("provider identity fields are required"))
    } else {
        Ok(())
    }
}
pub(crate) fn matches_query(
    query: &Query,
    company: &CompanyId,
    filed: chrono::NaiveDate,
    form: &str,
) -> bool {
    query.company == *company
        && (query.filed_from..=query.filed_to).contains(&filed)
        && (query.forms.is_empty() || query.forms.iter().any(|f| f == form))
}
impl SqliteRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        let transaction = connection.transaction()?;
        let version: i64 = transaction.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        match version {
            0 => {
                transaction.execute_batch(include_str!("schema.sql"))?;
                transaction.execute_batch(include_str!("market-v2.sql"))?;
            }
            1 => transaction.execute_batch(include_str!("market-v2.sql"))?,
            2..=4 => {}
            _ => {
                return Err(Error::new(
                    ErrorKind::Persistence,
                    "unsupported SQLite schema version",
                ));
            }
        };
        if version < 3 {
            transaction.execute_batch(include_str!("catalog-v3.sql"))?;
        }
        if version < 4 {
            transaction.execute_batch(include_str!("selection-v4.sql"))?;
        }
        transaction.commit()?;
        Ok(Self { connection })
    }
    fn active_run(&self, id: i64, operation: &str) -> Result<(String, Query)> {
        let (provider, query, status, op): (String, String, String, String) =
            self.connection.query_row(
                "SELECT provider_id,query,status,operation FROM runs WHERE id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        if status != "running" || (op != "both" && op != operation) {
            return Err(invalid(
                "run is inactive or does not include this capability",
            ));
        }
        Ok((provider, serde_json::from_str(&query)?))
    }
    fn save_page(
        &mut self,
        run: i64,
        provider: &str,
        kind: &str,
        rows: Vec<(String, String, Option<String>, String)>,
        cursor: &Option<String>,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        for (hash, payload, metric, retrieved) in rows {
            tx.execute("INSERT OR IGNORE INTO observations(provider_id,kind,fingerprint,payload,metric) VALUES(?,?,?,?,?)",params![provider,kind,hash,payload,metric])?;
            let id: i64 = tx.query_row(
                "SELECT id FROM observations WHERE provider_id=? AND kind=? AND fingerprint=?",
                params![provider, kind, hash],
                |r| r.get(0),
            )?;
            tx.execute("INSERT OR IGNORE INTO run_observations(run_id,observation_id,retrieved_at) VALUES(?,?,?)",params![run,id,retrieved])?;
        }
        let sql = if kind == "filing" {
            "UPDATE runs SET filings_cursor=? WHERE id=? AND status='running'"
        } else {
            "UPDATE runs SET facts_cursor=? WHERE id=? AND status='running'"
        };
        if tx.execute(sql, params![cursor, run])? != 1 {
            return Err(invalid("run is inactive"));
        }
        tx.commit()?;
        Ok(())
    }
    pub fn run_observations(&self, run: i64) -> Result<Vec<ObservationRetrieval>> {
        let mut statement = self.connection.prepare("SELECT o.id,o.kind,o.fingerprint,r.retrieved_at FROM run_observations r JOIN observations o ON o.id=r.observation_id WHERE r.run_id=? ORDER BY o.id")?;
        let rows = statement.query_map([run], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (observation_id, kind, fingerprint, at) = row?;
            Ok(ObservationRetrieval {
                observation_id,
                kind,
                fingerprint,
                retrieved_at: at.parse().map_err(|_| {
                    Error::new(ErrorKind::Persistence, "invalid stored retrieval timestamp")
                })?,
            })
        })
        .collect()
    }
    pub fn document_observations(&self, checksum: &str) -> Result<Vec<DocumentObservation>> {
        let mut stmt=self.connection.prepare("SELECT p.identity,d.source_url,d.media_type,d.retrieved_at FROM document_observations d JOIN providers p ON p.id=d.provider_id WHERE checksum=? ORDER BY d.id")?;
        let rows = stmt.query_map([checksum], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (p, source_url, media_type, at) = row?;
            Ok(DocumentObservation {
                provider: serde_json::from_str(&p)?,
                checksum: checksum.into(),
                source_url,
                media_type,
                retrieved_at: at.parse().map_err(|_| {
                    Error::new(ErrorKind::Persistence, "invalid stored retrieval timestamp")
                })?,
            })
        })
        .collect()
    }
}
impl Repository for SqliteRepository {
    fn start_run(&mut self, p: &ProviderIdentity, q: &Query, operation: &str) -> Result<i64> {
        validate_identity(p)?;
        q.validate()?;
        if !["both", "filings", "facts"].contains(&operation) {
            return Err(invalid("unknown ingestion operation"));
        }
        let mut q = q.clone();
        q.cursor = None;
        let tx = self.connection.transaction()?;
        let provider = fingerprint(p)?;
        tx.execute(
            "INSERT OR IGNORE INTO providers(id,identity) VALUES(?,?)",
            params![provider, serde_json::to_string(p)?],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO companies(namespace,value) VALUES(?,?)",
            params![q.company.namespace, q.company.value],
        )?;
        tx.execute("INSERT INTO runs(provider_id,query,operation,status,started_at) VALUES(?,?,?,'running',?)",params![provider,serde_json::to_string(&q)?,operation,Utc::now().to_rfc3339()])?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }
    fn save_filings_page(&mut self, run: i64, page: &Page<Filing>) -> Result<()> {
        let (provider, q) = self.active_run(run, "filings")?;
        if page.items.len() > q.page_size {
            return Err(invalid("page exceeds requested size"));
        }
        let mut rows = Vec::new();
        for f in &page.items {
            f.validate()?;
            if !matches_query(&q, &f.company, f.filed, &f.form) {
                return Err(invalid("filing is outside query"));
            }
            rows.push((
                f.fingerprint()?,
                serde_json::to_string(f)?,
                None,
                f.retrieved_at.to_rfc3339(),
            ));
        }
        self.save_page(run, &provider, "filing", rows, &page.next_cursor)
    }
    fn save_facts_page(
        &mut self,
        run: i64,
        page: &Page<Fact>,
        mapper: &dyn Fn(&Fact) -> Option<Metric>,
    ) -> Result<()> {
        let (provider, q) = self.active_run(run, "facts")?;
        if page.items.len() > q.page_size {
            return Err(invalid("page exceeds requested size"));
        }
        let mut rows = Vec::new();
        for f in &page.items {
            f.validate()?;
            if !matches_query(&q, &f.company, f.filed, &f.form) {
                return Err(invalid("fact is outside query"));
            }
            let metric = mapper(f);
            let hash = fingerprint(&(f.fingerprint()?, &metric))?;
            rows.push((
                hash,
                serde_json::to_string(f)?,
                metric.map(|m| serde_json::to_string(&m)).transpose()?,
                f.retrieved_at.to_rfc3339(),
            ));
        }
        self.save_page(run, &provider, "fact", rows, &page.next_cursor)
    }
    fn finish_run(&mut self, run: i64, error: Option<&Error>) -> Result<()> {
        let n = self.connection.execute(
            "UPDATE runs SET status=?,error=?,finished_at=? WHERE id=? AND status='running'",
            params![
                if error.is_some() {
                    "failed"
                } else {
                    "complete"
                },
                error.map(ToString::to_string),
                Utc::now().to_rfc3339(),
                run
            ],
        )?;
        if n != 1 {
            return Err(invalid("run does not exist or is already finished"));
        }
        Ok(())
    }
    fn snapshot(&self, p: &ProviderIdentity, q: &Query) -> Result<Snapshot> {
        validate_identity(p)?;
        q.validate()?;
        let provider = fingerprint(p)?;
        let mut stmt=self.connection.prepare("SELECT id,query,operation,status,filings_cursor,facts_cursor,error FROM runs WHERE provider_id=? ORDER BY id")?;
        let rows = stmt.query_map([&provider], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut runs = Vec::new();
        for row in rows {
            let (id, query, operation, status, filings_cursor, facts_cursor, error) = row?;
            let query: Query = serde_json::from_str(&query)?;
            if query.company != q.company
                || query.filed_to < q.filed_from
                || query.filed_from > q.filed_to
                || (!q.forms.is_empty()
                    && !query.forms.is_empty()
                    && !q.forms.iter().any(|f| query.forms.contains(f)))
            {
                continue;
            }
            let status = match status.as_str() {
                "running" => RunStatus::Running,
                "complete" => RunStatus::Complete,
                "failed" => RunStatus::Failed,
                _ => return Err(Error::new(ErrorKind::Persistence, "invalid run status")),
            };
            runs.push(Run {
                id,
                provider: p.clone(),
                query,
                operation,
                status,
                filings_cursor,
                facts_cursor,
                error,
            });
        }
        let mut stmt = self.connection.prepare(
            "SELECT kind,payload,metric,id FROM observations WHERE provider_id=? ORDER BY id",
        )?;
        let rows = stmt.query_map([provider], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        let mut filings = Vec::new();
        let mut facts = Vec::new();
        for row in rows {
            let (kind, payload, metric, observation_id) = row?;
            if kind == "filing" {
                let f: Filing = serde_json::from_str(&payload)?;
                if matches_query(q, &f.company, f.filed, &f.form) {
                    filings.push(f)
                }
            } else {
                let fact: Fact = serde_json::from_str(&payload)?;
                if matches_query(q, &fact.company, fact.filed, &fact.form) {
                    facts.push(FactObservation {
                        observation_id,
                        fact,
                        metric: metric.map(|m| serde_json::from_str(&m)).transpose()?,
                    })
                }
            }
        }
        Ok(Snapshot {
            runs,
            filings,
            facts,
        })
    }
    fn save_document(
        &mut self,
        p: &ProviderIdentity,
        d: &Document,
        max_bytes: usize,
    ) -> Result<String> {
        validate_identity(p)?;
        let bytes = d.bytes(max_bytes)?;
        let checksum = format!("{:x}", Sha256::digest(&bytes));
        let provider = fingerprint(p)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO providers(id,identity) VALUES(?,?)",
            params![provider, serde_json::to_string(p)?],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO document_content(checksum,content) VALUES(?,?)",
            params![checksum, bytes],
        )?;
        tx.execute("INSERT INTO document_observations(provider_id,checksum,source_url,media_type,retrieved_at) VALUES(?,?,?,?,?)",params![provider,checksum,d.source_url,d.media_type,d.retrieved_at.to_rfc3339()])?;
        tx.commit()?;
        Ok(checksum)
    }
    fn stored_document(&self, checksum: &str) -> Result<Vec<u8>> {
        let bytes: Vec<u8> = self
            .connection
            .query_row(
                "SELECT content FROM document_content WHERE checksum=?",
                [checksum],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "document is not stored"))?;
        if format!("{:x}", Sha256::digest(&bytes)) != checksum {
            return Err(Error::new(
                ErrorKind::Persistence,
                "document checksum mismatch",
            ));
        }
        Ok(bytes)
    }
}

pub mod market;
