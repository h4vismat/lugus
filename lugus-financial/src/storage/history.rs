//! Immutable, exact-run historical snapshots with transactional cursor progress.
use super::bounded::{BoundedReadError, ReadLimits, ReadResult};
use super::{RunStatus, SqliteRepository, invalid, validate_identity};
use crate::{
    domain::{ProviderIdentity, Validate, fingerprint},
    error::{Error, ErrorKind, Result},
    historical_prices::*,
};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRun {
    pub id: i64,
    pub provider: ProviderIdentity,
    pub query: HistoryQuery,
    pub status: RunStatus,
    pub manifest: Option<HistoryManifest>,
    pub row_count: usize,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryObservation {
    pub id: i64,
    pub day: HistoryDay,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryReadPage {
    pub run: HistoryRun,
    pub items: Vec<HistoryObservation>,
    pub next_offset: Option<usize>,
}
pub trait HistoryRepository {
    fn start_history_run(&mut self, p: &ProviderIdentity, q: &HistoryQuery) -> Result<i64>;
    fn save_history_page(&mut self, run: i64, page: &HistoryPage) -> Result<()>;
    fn finish_history_run(&mut self, run: i64, error: Option<&Error>) -> Result<()>;
    fn history_run(
        &self,
        p: &ProviderIdentity,
        run: i64,
        limits: ReadLimits,
    ) -> ReadResult<HistoryRun>;
    fn history_page(
        &self,
        p: &ProviderIdentity,
        run: i64,
        offset: usize,
        limits: ReadLimits,
    ) -> ReadResult<HistoryReadPage>;
}
impl HistoryRepository for SqliteRepository {
    fn start_history_run(&mut self, p: &ProviderIdentity, q: &HistoryQuery) -> Result<i64> {
        validate_identity(p)?;
        q.validate()?;
        let mut q = q.clone();
        q.cursor = None;
        let tx = self.connection.transaction()?;
        let key = fingerprint(p)?;
        tx.execute(
            "INSERT OR IGNORE INTO providers(id,identity) VALUES(?1,?2)",
            params![key, serde_json::to_string(p)?],
        )?;
        tx.execute("INSERT INTO historical_runs(provider_id,query,status,started_at) VALUES(?1,?2,'running',?3)",params![key,serde_json::to_string(&q)?,Utc::now().to_rfc3339()])?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }
    fn save_history_page(&mut self, run: i64, page: &HistoryPage) -> Result<()> {
        let tx = self.connection.transaction()?;
        let (provider,raw_query,status,old_manifest,cursor,seen,last_date,count):(String,String,String,Option<String>,Option<String>,String,Option<String>,i64)=tx.query_row(
            "SELECT provider_id,query,status,manifest,cursor,seen_cursors,last_date,row_count FROM historical_runs WHERE id=?1",[run],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)))?;
        if status != "running" || (old_manifest.is_some() && cursor.is_none()) {
            return Err(invalid("historical run is inactive or exhausted"));
        }
        let mut q: HistoryQuery = serde_json::from_str(&raw_query)?;
        q.cursor = cursor;
        page.validate_for(&q)?;
        if let Some(m) = old_manifest {
            if serde_json::from_str::<HistoryManifest>(&m)? != page.manifest {
                return Err(invalid("historical manifest changed across pages"));
            }
        }
        if let Some(last) = last_date {
            let last = last
                .parse::<chrono::NaiveDate>()
                .map_err(|_| invalid("invalid stored historical date"))?;
            if last.succ_opt() != Some(page.items[0].date) {
                return Err(invalid("historical dates did not advance consecutively"));
            }
        }
        let count = usize::try_from(count).map_err(|_| invalid("invalid historical row count"))?;
        if count > 100_000 || page.items.len() > 100_000 - count {
            return Err(invalid("historical run exceeds observation limit"));
        }
        let mut seen: Vec<String> = serde_json::from_str(&seen)?;
        if let Some(c) = &page.next_cursor {
            if seen.contains(c) {
                return Err(invalid("historical cursor repeated"));
            }
            seen.push(c.clone());
        }
        for (index, day) in page.items.iter().enumerate() {
            let hash = fingerprint(&(day, &page.manifest))?;
            tx.execute("INSERT OR IGNORE INTO historical_observations(provider_id,fingerprint,payload) VALUES(?1,?2,?3)",params![provider,hash,serde_json::to_string(day)?])?;
            let id: i64 = tx.query_row(
                "SELECT id FROM historical_observations WHERE provider_id=?1 AND fingerprint=?2",
                params![provider, hash],
                |r| r.get(0),
            )?;
            tx.execute("INSERT INTO historical_run_days(run_id,ordinal,date,observation_id) VALUES(?1,?2,?3,?4)",params![run,(count+index) as i64,day.date.to_string(),id])?;
        }
        tx.execute("UPDATE historical_runs SET manifest=?1,cursor=?2,seen_cursors=?3,last_date=?4,row_count=?5 WHERE id=?6",params![serde_json::to_string(&page.manifest)?,page.next_cursor,serde_json::to_string(&seen)?,page.items.last().unwrap().date.to_string(),(count+page.items.len()) as i64,run])?;
        tx.commit()?;
        Ok(())
    }
    fn finish_history_run(&mut self, run: i64, error: Option<&Error>) -> Result<()> {
        let changed=self.connection.execute("UPDATE historical_runs SET status=?1,error=?2,finished_at=?3 WHERE id=?4 AND status='running' AND (?5 OR (manifest IS NOT NULL AND cursor IS NULL AND last_date=json_extract(manifest,'$.anchor')))",
            params![if error.is_some(){"failed"}else{"complete"},error.map(|e|e.to_string().chars().take(2048).collect::<String>()),Utc::now().to_rfc3339(),run,error.is_some()])?;
        if changed != 1 {
            return Err(invalid("historical run is inactive or incomplete"));
        }
        Ok(())
    }
    fn history_run(
        &self,
        p: &ProviderIdentity,
        run: i64,
        limits: ReadLimits,
    ) -> ReadResult<HistoryRun> {
        limits.validate()?;
        validate_identity(p)?;
        let tx = self.connection.unchecked_transaction()?;
        let result = read_run(&tx, p, run, limits.max_bytes)?;
        tx.commit()?;
        Ok(result)
    }
    fn history_page(
        &self,
        p: &ProviderIdentity,
        run: i64,
        offset: usize,
        limits: ReadLimits,
    ) -> ReadResult<HistoryReadPage> {
        limits.validate()?;
        validate_identity(p)?;
        if limits.max_items > 200 || offset > 100_000 {
            return Err(invalid("historical read page exceeds bounds").into());
        }
        let tx = self.connection.unchecked_transaction()?;
        let header = read_run(&tx, p, run, limits.max_bytes)?;
        let meta_bytes = serde_json::to_vec(&header).map_err(Error::from)?.len();
        let size:i64=tx.query_row("SELECT coalesce(sum(n),0) FROM (SELECT length(CAST(o.payload AS BLOB))+64 AS n FROM historical_run_days d JOIN historical_observations o ON o.id=d.observation_id WHERE d.run_id=?1 AND d.ordinal>=?2 ORDER BY d.ordinal LIMIT ?3)",params![run,offset as i64,limits.max_items as i64],|r|r.get(0))?;
        if size < 0 || size as usize > limits.max_bytes.saturating_sub(meta_bytes) {
            return Err(BoundedReadError::LimitExceeded);
        }
        let mut stmt=tx.prepare("SELECT o.id,o.payload FROM historical_run_days d JOIN historical_observations o ON o.id=d.observation_id WHERE d.run_id=?1 AND d.ordinal>=?2 ORDER BY d.ordinal LIMIT ?3")?;
        let rows = stmt.query_map(params![run, offset as i64, limits.max_items as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        let items = rows
            .map(|r| {
                let (id, json) = r?;
                Ok(HistoryObservation {
                    id,
                    day: serde_json::from_str(&json)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        drop(stmt);
        let next_offset = (offset + items.len() < header.row_count).then_some(offset + items.len());
        tx.commit()?;
        Ok(HistoryReadPage {
            run: header,
            items,
            next_offset,
        })
    }
}
fn read_run(
    c: &rusqlite::Connection,
    p: &ProviderIdentity,
    id: i64,
    max: usize,
) -> ReadResult<HistoryRun> {
    let key = fingerprint(p)?;
    let size:Option<i64>=c.query_row("SELECT length(CAST(query AS BLOB))+coalesce(length(CAST(manifest AS BLOB)),0)+coalesce(length(CAST(error AS BLOB)),0)+512 FROM historical_runs WHERE id=?1 AND provider_id=?2",params![id,key],|r|r.get(0)).optional()?;
    let size = size
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "historical run not found for provider"))?;
    if size < 0 || size as usize > max {
        return Err(BoundedReadError::LimitExceeded);
    }
    let (q,status,m,count,error):(String,String,Option<String>,i64,Option<String>)=c.query_row("SELECT query,status,manifest,row_count,error FROM historical_runs WHERE id=?1 AND provider_id=?2",params![id,key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    let status = match status.as_str() {
        "running" => RunStatus::Running,
        "complete" => RunStatus::Complete,
        "failed" => RunStatus::Failed,
        _ => {
            return Err(Error::new(ErrorKind::Persistence, "invalid historical run status").into());
        }
    };
    Ok(HistoryRun {
        id,
        provider: p.clone(),
        query: serde_json::from_str(&q).map_err(Error::from)?,
        status,
        manifest: m
            .map(|v| serde_json::from_str(&v))
            .transpose()
            .map_err(Error::from)?,
        row_count: usize::try_from(count).map_err(|_| invalid("invalid historical row count"))?,
        error,
    })
}
