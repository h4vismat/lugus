//! Append-only identity evidence. Catalog reads never invoke a provider.
use super::{Candidate, MatchReason, ResolutionPage, SearchQuery, SearchRequest};
use crate::{
    domain::{ProviderIdentity, Validate},
    error::{Error, ErrorKind, Result},
    storage::SqliteRepository,
};
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CatalogCompanyId(pub i64);
/// A user choice is distinct from both catalog identity and source observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CatalogSelectionId(pub i64);
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogSelection {
    pub id: CatalogSelectionId,
    pub entry: CatalogEntry,
    pub run_status: crate::storage::RunStatus,
    pub snapshot: Option<String>,
    pub coverage: Option<String>,
    pub error: Option<String>,
    pub failure: Option<ResolutionFailure>,
    pub selected_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionFailure {
    pub kind: ErrorKind,
    pub message: String,
    pub retry_after_seconds: Option<u64>,
}
impl From<&Error> for ResolutionFailure {
    fn from(error: &Error) -> Self {
        Self {
            kind: error.kind,
            message: error.message.clone(),
            retry_after_seconds: error.retry_after_seconds,
        }
    }
}
/// A selected source retrieval, including the host-attached provider identity.
pub type CatalogEntry = CatalogRetrieval;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogRetrieval {
    pub company: CatalogCompanyId,
    pub candidate: Candidate,
    pub provider: ProviderIdentity,
    pub run_id: i64,
    pub observation_id: i64,
    pub recorded_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ResolutionOutcome {
    Resolved {
        entry: Box<CatalogEntry>,
        snapshot: String,
        coverage: String,
    },
    Candidates {
        items: Vec<CatalogEntry>,
        snapshot: String,
        coverage: String,
    },
    NoMatch {
        snapshot: String,
        coverage: String,
    },
    Incomplete {
        items: Vec<CatalogEntry>,
        error: Option<String>,
        failure: Option<ResolutionFailure>,
        snapshot: Option<String>,
        coverage: Option<String>,
    },
    IdentityConflict {
        items: Vec<CatalogEntry>,
        snapshot: String,
        coverage: String,
    },
}
pub trait CatalogRepository {
    fn start_resolution_run(
        &mut self,
        provider: &ProviderIdentity,
        request: &SearchRequest,
    ) -> Result<i64>;
    fn save_resolution_page(
        &mut self,
        run: i64,
        request: &SearchRequest,
        page: &ResolutionPage,
    ) -> Result<()>;
    fn fail_resolution_run(&mut self, run: i64, message: &str) -> Result<()>;
    fn fail_resolution_run_with_error(&mut self, run: i64, error: &Error) -> Result<()>;
    fn select_candidate(&mut self, run: i64, observation: i64) -> Result<CatalogSelection>;
    fn catalog_selection(&self, selection: CatalogSelectionId) -> Result<CatalogSelection>;
    fn resolution_outcome(&self, run: i64) -> Result<ResolutionOutcome>;
    fn search_catalog(&self, query: &SearchQuery) -> Result<Vec<CatalogEntry>>;
    fn catalog_history(&self, company: CatalogCompanyId) -> Result<Vec<CatalogRetrieval>>;
}
fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidRequest, message)
}
impl SqliteRepository {
    pub(crate) fn catalog_records(
        &self,
        run: Option<i64>,
        company: Option<CatalogCompanyId>,
    ) -> Result<Vec<CatalogRetrieval>> {
        let mut stmt=self.connection.prepare("SELECT o.company_id,o.payload,o.provider,p.run_id,o.id,r.recorded_at,r.retrieved_at,r.match_reasons FROM catalog_retrievals r JOIN catalog_observations o ON o.id=r.observation_id JOIN resolution_pages p ON p.id=r.page_id WHERE (?1 IS NULL OR p.run_id=?1) AND (?2 IS NULL OR o.company_id=?2) ORDER BY r.id")?;
        let rows = stmt.query_map(params![run, company.map(|id| id.0)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        rows.map(|r| {
            let (id, payload, provider, run_id, observation_id, recorded, retrieved, reasons) = r?;
            let mut candidate: Candidate = serde_json::from_str(&payload)?;
            candidate.retrieved_at = serde_json::from_str(&retrieved)?;
            candidate.match_reasons = serde_json::from_str(&reasons)?;
            Ok(CatalogRetrieval {
                company: CatalogCompanyId(id),
                candidate,
                provider: serde_json::from_str(&provider)?,
                run_id,
                observation_id,
                recorded_at: serde_json::from_str(&recorded)?,
            })
        })
        .collect()
    }
}
fn entries(records: Vec<CatalogRetrieval>, query: &SearchQuery) -> Vec<CatalogEntry> {
    let mut result = BTreeMap::new();
    for mut record in records {
        record.candidate.match_reasons = super::match_reasons(query, &record.candidate);
        if !record.candidate.match_reasons.is_empty() {
            result.insert(record.company, record);
        }
    }
    let mut result: Vec<CatalogEntry> = result.into_values().collect();
    result.sort_by_key(|entry| {
        (
            !entry
                .candidate
                .match_reasons
                .contains(&MatchReason::ExactName),
            super::normalized_name(&entry.candidate.name),
            entry.candidate.identifier.namespace.clone(),
            entry.candidate.identifier.value.clone(),
            entry.company,
        )
    });
    result
}
impl CatalogRepository for SqliteRepository {
    fn start_resolution_run(
        &mut self,
        provider: &ProviderIdentity,
        request: &SearchRequest,
    ) -> Result<i64> {
        request.validate()?;
        if request.cursor.is_some() {
            return Err(invalid("new resolution run requires a root query"));
        }
        if [
            &provider.instance_id,
            &provider.plugin_id,
            &provider.plugin_version,
        ]
        .iter()
        .any(|s| s.trim().is_empty())
        {
            return Err(invalid("provider identity is required"));
        }
        self.connection.execute("INSERT INTO resolution_runs(provider,request,status,started_at) VALUES(?,?,'running',?)",params![serde_json::to_string(provider)?,serde_json::to_string(request)?,serde_json::to_string(&Utc::now())?])?;
        Ok(self.connection.last_insert_rowid())
    }
    fn save_resolution_page(
        &mut self,
        run: i64,
        request: &SearchRequest,
        page: &ResolutionPage,
    ) -> Result<()> {
        request.validate()?;
        page.validate_for(request)?;
        let tx = self.connection.transaction()?;
        let (provider_json,root,status,snapshot,coverage,cursor,seen):(String,String,String,Option<String>,Option<String>,Option<String>,String)=tx.query_row("SELECT provider,request,status,snapshot,coverage,cursor,seen_cursors FROM resolution_runs WHERE id=?",[run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)))?;
        let mut expected: SearchRequest = serde_json::from_str(&root)?;
        expected.cursor = cursor;
        if status != "running" || serde_json::to_value(&expected)? != serde_json::to_value(request)?
        {
            return Err(invalid("inactive run or mismatched continuation request"));
        }
        if snapshot.as_ref().is_some_and(|s| s != &page.snapshot)
            || coverage.as_ref().is_some_and(|s| s != &page.coverage)
        {
            return Err(invalid("resolution source snapshot or coverage changed"));
        }
        let mut seen: Vec<String> = serde_json::from_str(&seen)?;
        if let Some(next) = &page.next_cursor {
            if seen.contains(next) {
                return Err(invalid("repeated resolution cursor"));
            }
            seen.push(next.clone());
        }
        let now = serde_json::to_string(&Utc::now())?;
        tx.execute("INSERT INTO resolution_pages(run_id,request_cursor,next_cursor,recorded_at) VALUES(?,?,?,?)",params![run,request.cursor,page.next_cursor,now])?;
        let page_id = tx.last_insert_rowid();
        let provider: ProviderIdentity = serde_json::from_str(&provider_json)?;
        for candidate in &page.items {
            candidate.validate()?;
            let duplicate:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM catalog_retrievals r JOIN resolution_pages p ON p.id=r.page_id JOIN catalog_observations o ON o.id=r.observation_id JOIN catalog_companies c ON c.id=o.company_id WHERE p.run_id=? AND c.identifier=?)",params![run,serde_json::to_string(&candidate.identifier)?],|r|r.get(0))?;
            if duplicate {
                return Err(invalid("duplicate candidate across resolution pages"));
            }
            let scope = if candidate.identifier.namespace == "sec:cik" {
                "sec:cik".to_owned()
            } else {
                serde_json::to_string(&(&provider.plugin_id, &provider.instance_id))?
            };
            let identifier = serde_json::to_string(&candidate.identifier)?;
            tx.execute(
                "INSERT OR IGNORE INTO catalog_companies(identity_scope,identifier) VALUES(?,?)",
                params![scope, identifier],
            )?;
            let company: i64 = tx.query_row(
                "SELECT id FROM catalog_companies WHERE identity_scope=? AND identifier=?",
                params![scope, identifier],
                |r| r.get(0),
            )?;
            let mut payload = serde_json::to_value(candidate)?;
            payload.as_object_mut().unwrap().remove("retrieved_at");
            payload.as_object_mut().unwrap().remove("match_reasons");
            // Retain a parseable candidate payload while excluding retrieval-specific fields from identity.
            let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&payload)?));
            tx.execute("INSERT OR IGNORE INTO catalog_observations(company_id,provider,fingerprint,payload) VALUES(?,?,?,?)",params![company,provider_json,fingerprint,serde_json::to_string(candidate)?])?;
            let observation:i64=tx.query_row("SELECT id FROM catalog_observations WHERE company_id=? AND provider=? AND fingerprint=?",params![company,provider_json,fingerprint],|r|r.get(0))?;
            tx.execute("INSERT INTO catalog_retrievals(page_id,observation_id,retrieved_at,recorded_at,match_reasons) VALUES(?,?,?,?,?)",params![page_id,observation,serde_json::to_string(&candidate.retrieved_at)?,now,serde_json::to_string(&candidate.match_reasons)?])?;
        }
        tx.execute("UPDATE resolution_runs SET snapshot=?,coverage=?,cursor=?,seen_cursors=?,status=?,finished_at=? WHERE id=?",params![page.snapshot,page.coverage,page.next_cursor,serde_json::to_string(&seen)?,if page.next_cursor.is_none(){"complete"}else{"running"},if page.next_cursor.is_none(){Some(&now)}else{None},run])?;
        tx.commit()?;
        Ok(())
    }
    fn fail_resolution_run(&mut self, run: i64, message: &str) -> Result<()> {
        self.fail_resolution_run_with_error(run, &Error::new(ErrorKind::Unavailable, message))
    }
    fn fail_resolution_run_with_error(&mut self, run: i64, error: &Error) -> Result<()> {
        let count=self.connection.execute("UPDATE resolution_runs SET status='failed',error=?,failure=?,finished_at=? WHERE id=? AND status='running'",params![error.message,serde_json::to_string(&ResolutionFailure::from(error))?,serde_json::to_string(&Utc::now())?,run])?;
        if count != 1 {
            return Err(invalid("resolution run is not active"));
        }
        Ok(())
    }
    fn select_candidate(&mut self, run: i64, observation: i64) -> Result<CatalogSelection> {
        let tx = self.connection.transaction()?;
        let (retrieval,payload,provider,company,recorded,retrieved,reasons):(i64,String,String,i64,String,String,String)=tx.query_row("SELECT r.id,o.payload,o.provider,o.company_id,r.recorded_at,r.retrieved_at,r.match_reasons FROM catalog_retrievals r JOIN catalog_observations o ON o.id=r.observation_id JOIN resolution_pages p ON p.id=r.page_id WHERE p.run_id=? AND o.id=?",params![run,observation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?.ok_or_else(||invalid("observation does not belong to resolution run"))?;
        let mut candidate: Candidate = serde_json::from_str(&payload)?;
        candidate.retrieved_at = serde_json::from_str(&retrieved)?;
        candidate.match_reasons = serde_json::from_str(&reasons)?;
        let entry = CatalogEntry {
            company: CatalogCompanyId(company),
            candidate,
            provider: serde_json::from_str(&provider)?,
            run_id: run,
            observation_id: observation,
            recorded_at: serde_json::from_str(&recorded)?,
        };
        tx.execute("INSERT INTO catalog_selections(retrieval_id,entry,run_status,snapshot,coverage,error,failure,selected_at) SELECT ?,?,status,snapshot,coverage,error,failure,? FROM resolution_runs WHERE id=?",params![retrieval,serde_json::to_string(&entry)?,serde_json::to_string(&Utc::now())?,run])?;
        let id = CatalogSelectionId(tx.last_insert_rowid());
        tx.commit()?;
        self.catalog_selection(id)
    }
    fn catalog_selection(&self, selection: CatalogSelectionId) -> Result<CatalogSelection> {
        struct SelectionRow {
            entry: String,
            status: String,
            snapshot: Option<String>,
            coverage: Option<String>,
            error: Option<String>,
            failure: Option<String>,
            selected: String,
        }
        let SelectionRow{entry,status,snapshot,coverage,error,failure,selected}=self.connection.query_row("SELECT entry,run_status,snapshot,coverage,error,failure,selected_at FROM catalog_selections WHERE id=?",[selection.0],|r|Ok(SelectionRow{entry:r.get(0)?,status:r.get(1)?,snapshot:r.get(2)?,coverage:r.get(3)?,error:r.get(4)?,failure:r.get(5)?,selected:r.get(6)?})).optional()?.ok_or_else(||invalid("unknown catalog selection"))?;
        Ok(CatalogSelection {
            id: selection,
            entry: serde_json::from_str(&entry)?,
            run_status: serde_json::from_value(serde_json::Value::String(status))?,
            snapshot,
            coverage,
            error,
            failure: failure.map(|v| serde_json::from_str(&v)).transpose()?,
            selected_at: serde_json::from_str(&selected)?,
        })
    }
    fn resolution_outcome(&self, run: i64) -> Result<ResolutionOutcome> {
        let (request, status, snapshot, coverage, error): (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = self
            .connection
            .query_row(
                "SELECT request,status,snapshot,coverage,error FROM resolution_runs WHERE id=?",
                [run],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?
            .ok_or_else(|| invalid("unknown resolution run"))?;
        let request: SearchRequest = serde_json::from_str(&request)?;
        let items = entries(self.catalog_records(Some(run), None)?, &request.query);
        if status != "complete" {
            return Ok(ResolutionOutcome::Incomplete {
                items,
                error,
                failure: self
                    .connection
                    .query_row(
                        "SELECT failure FROM resolution_runs WHERE id=?",
                        [run],
                        |r| r.get::<_, Option<String>>(0),
                    )?
                    .map(|v| serde_json::from_str(&v))
                    .transpose()?,
                snapshot,
                coverage,
            });
        }
        let snapshot = snapshot.ok_or_else(|| invalid("complete run lacks snapshot"))?;
        let coverage = coverage.ok_or_else(|| invalid("complete run lacks coverage"))?;
        if items.is_empty() {
            return Ok(ResolutionOutcome::NoMatch { snapshot, coverage });
        }
        if matches!(request.query, SearchQuery::Identifier { .. })
            && items.len() == 1
            && items[0]
                .candidate
                .match_reasons
                .iter()
                .any(|r| matches!(r, MatchReason::ExactIdentifier))
        {
            // Unknown namespaces have provider-local identity semantics. They cannot
            // conflict solely because another provider uses the same external string.
            let global = matches!(&request.query, SearchQuery::Identifier{identifier,..} if matches!(identifier.namespace.as_str(), "sec:cik"|"sec:ticker"|"sec:exchange"));
            let known = self
                .search_catalog(&request.query)?
                .into_iter()
                .filter(|entry| {
                    global
                        || (entry.provider.plugin_id == items[0].provider.plugin_id
                            && entry.provider.instance_id == items[0].provider.instance_id)
                })
                .collect::<Vec<_>>();
            if known.iter().any(|k| k.company != items[0].company) {
                return Ok(ResolutionOutcome::IdentityConflict {
                    items: known,
                    snapshot,
                    coverage,
                });
            }
            let entry = items.into_iter().next().unwrap();
            return Ok(ResolutionOutcome::Resolved {
                entry: Box::new(entry),
                snapshot,
                coverage,
            });
        }
        Ok(ResolutionOutcome::Candidates {
            items,
            snapshot,
            coverage,
        })
    }
    fn search_catalog(&self, query: &SearchQuery) -> Result<Vec<CatalogEntry>> {
        SearchRequest {
            query: query.clone(),
            page_size: 100,
            cursor: None,
        }
        .validate()?;
        Ok(entries(self.catalog_records(None, None)?, query))
    }
    fn catalog_history(&self, company: CatalogCompanyId) -> Result<Vec<CatalogRetrieval>> {
        self.catalog_records(None, Some(company))
    }
}
