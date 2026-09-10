//! Immutable daily price revisions and explicit, offline ingestion history.
use super::{ObservationRetrieval, RunStatus, SqliteRepository, invalid, validate_identity};
use crate::{
    domain::{ProviderIdentity, Validate, fingerprint},
    error::{Error, ErrorKind, Result},
    market_data::*,
};
use chrono::{NaiveDate, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketRun {
    pub id: i64,
    pub provider: ProviderIdentity,
    pub query: PriceQuery,
    pub status: RunStatus,
    pub cursor: Option<String>,
    /// Whole source snapshot coverage, not evidence of successful page ingestion.
    pub coverage: Option<PriceCoverage>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceObservation {
    pub observation_id: i64,
    pub bar: PriceBar,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSnapshot {
    pub runs: Vec<MarketRun>,
    pub prices: Vec<PriceObservation>,
}
pub trait MarketRepository {
    fn start_market_run(&mut self, provider: &ProviderIdentity, query: &PriceQuery) -> Result<i64>;
    fn save_prices_page(&mut self, run: i64, page: &PricePage) -> Result<()>;
    fn finish_market_run(&mut self, run: i64, error: Option<&Error>) -> Result<()>;
    fn market_snapshot(
        &self,
        provider: &ProviderIdentity,
        query: &PriceQuery,
    ) -> Result<MarketSnapshot>;
    fn market_run_observations(&self, run: i64) -> Result<Vec<ObservationRetrieval>>;
}
impl MarketRepository for SqliteRepository {
    fn start_market_run(&mut self, p: &ProviderIdentity, q: &PriceQuery) -> Result<i64> {
        validate_identity(p)?;
        q.validate()?;
        let mut q = q.clone();
        q.cursor = None;
        let tx = self.connection.transaction()?;
        let provider = fingerprint(p)?;
        tx.execute(
            "INSERT OR IGNORE INTO providers(id,identity) VALUES(?,?)",
            params![provider, serde_json::to_string(p)?],
        )?;
        tx.execute(
            "INSERT INTO market_runs(provider_id,query,status,started_at) VALUES(?,?,'running',?)",
            params![
                provider,
                serde_json::to_string(&q)?,
                Utc::now().to_rfc3339()
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }
    fn save_prices_page(&mut self, run: i64, page: &PricePage) -> Result<()> {
        let tx = self.connection.transaction()?;
        let (provider,query,status,cursor,coverage,last_date,seen):(String,String,String,Option<String>,Option<String>,Option<String>,String)=tx.query_row("SELECT provider_id,query,status,cursor,coverage,last_date,seen_cursors FROM market_runs WHERE id=?",[run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)))?;
        if status != "running" || (coverage.is_some() && cursor.is_none()) {
            return Err(invalid("market run inactive or final page already saved"));
        }
        let mut query: PriceQuery = serde_json::from_str(&query)?;
        query.cursor = cursor;
        page.validate_for(&query)?;
        if let Some(coverage) = coverage {
            let previous: PriceCoverage = serde_json::from_str(&coverage)?;
            if previous != page.coverage {
                return Err(invalid("market coverage changed across pages"));
            }
        }
        if let Some(date) = last_date {
            let previous: NaiveDate = date
                .parse()
                .map_err(|_| Error::new(ErrorKind::Persistence, "invalid stored price date"))?;
            if page.items.first().is_some_and(|bar| bar.date <= previous) {
                return Err(invalid("market dates did not advance across pages"));
            }
        }
        let mut seen: Vec<String> = serde_json::from_str(&seen)?;
        if let Some(cursor) = &page.next_cursor {
            if seen.contains(cursor) {
                return Err(invalid("market cursor repeated"));
            }
            seen.push(cursor.clone());
        }
        for bar in &page.items {
            let hash = bar.fingerprint()?;
            tx.execute("INSERT OR IGNORE INTO price_observations(provider_id,fingerprint,payload) VALUES(?,?,?)",params![provider,hash,serde_json::to_string(bar)?])?;
            let id: i64 = tx.query_row(
                "SELECT id FROM price_observations WHERE provider_id=? AND fingerprint=?",
                params![provider, hash],
                |r| r.get(0),
            )?;
            tx.execute("INSERT INTO market_run_observations(run_id,observation_id,retrieved_at) VALUES(?,?,?)",params![run,id,bar.retrieved_at.to_rfc3339()])?;
        }
        tx.execute(
            "UPDATE market_runs SET cursor=?,coverage=?,last_date=?,seen_cursors=? WHERE id=?",
            params![
                page.next_cursor,
                serde_json::to_string(&page.coverage)?,
                page.items.last().map(|bar| bar.date.to_string()),
                serde_json::to_string(&seen)?,
                run
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    fn finish_market_run(&mut self, run: i64, error: Option<&Error>) -> Result<()> {
        let changed=self.connection.execute("UPDATE market_runs SET status=?,error=?,finished_at=? WHERE id=? AND status='running' AND (? OR (coverage IS NOT NULL AND cursor IS NULL))",params![if error.is_some(){"failed"}else{"complete"},error.map(ToString::to_string),Utc::now().to_rfc3339(),run,error.is_some()])?;
        if changed != 1 {
            return Err(invalid("market run inactive or pages not exhausted"));
        }
        Ok(())
    }
    fn market_snapshot(&self, p: &ProviderIdentity, q: &PriceQuery) -> Result<MarketSnapshot> {
        validate_identity(p)?;
        q.validate()?;
        let provider = fingerprint(p)?;
        let mut statement=self.connection.prepare("SELECT id,query,status,cursor,coverage,error FROM market_runs WHERE provider_id=? ORDER BY id")?;
        let rows = statement.query_map([&provider], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut runs = vec![];
        for row in rows {
            let (id, query, status, cursor, coverage, error) = row?;
            let query: PriceQuery = serde_json::from_str(&query)?;
            if query.instrument != q.instrument || query.end < q.start || query.start > q.end {
                continue;
            }
            let status = match status.as_str() {
                "running" => RunStatus::Running,
                "complete" => RunStatus::Complete,
                "failed" => RunStatus::Failed,
                _ => {
                    return Err(Error::new(
                        ErrorKind::Persistence,
                        "invalid market run status",
                    ));
                }
            };
            runs.push(MarketRun {
                id,
                provider: p.clone(),
                query,
                status,
                cursor,
                coverage: coverage.map(|v| serde_json::from_str(&v)).transpose()?,
                error,
            });
        }
        let mut statement = self
            .connection
            .prepare("SELECT id,payload FROM price_observations WHERE provider_id=? ORDER BY id")?;
        let rows = statement.query_map([provider], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut prices = vec![];
        for row in rows {
            let (observation_id, payload) = row?;
            let bar: PriceBar = serde_json::from_str(&payload)?;
            if bar.instrument == q.instrument && (q.start..=q.end).contains(&bar.date) {
                prices.push(PriceObservation {
                    observation_id,
                    bar,
                });
            }
        }
        prices.sort_by_key(|p| (p.bar.date, p.observation_id));
        Ok(MarketSnapshot { runs, prices })
    }
    fn market_run_observations(&self, run: i64) -> Result<Vec<ObservationRetrieval>> {
        let mut statement=self.connection.prepare("SELECT o.id,o.fingerprint,r.retrieved_at FROM market_run_observations r JOIN price_observations o ON o.id=r.observation_id WHERE r.run_id=? ORDER BY o.id")?;
        let rows = statement.query_map([run], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (observation_id, fingerprint, at) = row?;
            Ok(ObservationRetrieval {
                observation_id,
                kind: "price".into(),
                fingerprint,
                retrieved_at: at.parse().map_err(|_| {
                    Error::new(ErrorKind::Persistence, "invalid stored retrieval timestamp")
                })?,
            })
        })
        .collect()
    }
}
