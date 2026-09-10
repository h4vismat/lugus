//! Additive run-scoped read port. Retrieval times belong to run associations.
use super::*;
use crate::{market_data::*, selection::*, storage::market::MarketRun};
fn status(value: &str) -> Result<RunStatus> {
    match value {
        "running" => Ok(RunStatus::Running),
        "complete" => Ok(RunStatus::Complete),
        "failed" => Ok(RunStatus::Failed),
        _ => Err(Error::new(ErrorKind::Persistence, "invalid run status")),
    }
}
fn timestamp(value: &str) -> Result<chrono::DateTime<Utc>> {
    value
        .parse()
        .map_err(|_| Error::new(ErrorKind::Persistence, "invalid run timestamp"))
}
impl SqliteRepository {
    fn selection_context(
        &self,
        kind: &str,
        id: i64,
        started: &str,
        finished: Option<&str>,
    ) -> Result<RunContext> {
        let (repository_id,sequence)=self.connection.query_row("SELECT i.identity,c.sequence FROM repository_identity i, ingestion_chronology c WHERE i.singleton=1 AND c.kind=? AND c.run_id=?",params![kind,id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        Ok(RunContext {
            repository_id,
            sequence,
            started_at: timestamp(started)?,
            finished_at: finished.map(timestamp).transpose()?,
        })
    }
    fn selection_observations<T: serde::de::DeserializeOwned>(
        &self,
        run: i64,
        kind: &str,
        market: bool,
    ) -> Result<Vec<Evidence<T>>> {
        let sql = if market {
            "SELECT o.id,o.fingerprint,r.retrieved_at,o.payload FROM market_run_observations r JOIN price_observations o ON o.id=r.observation_id WHERE r.run_id=? ORDER BY o.id"
        } else {
            "SELECT o.id,o.fingerprint,r.retrieved_at,o.payload FROM run_observations r JOIN observations o ON o.id=r.observation_id WHERE r.run_id=? AND o.kind=? ORDER BY o.id"
        };
        let mut stmt = self.connection.prepare(sql)?;
        let decode = |r: &rusqlite::Row<'_>| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        };
        let rows = if market {
            stmt.query_map(params![run], decode)?
        } else {
            stmt.query_map(params![run, kind], decode)?
        };
        rows.map(|row| {
            let (observation_id, fingerprint, at, payload) = row?;
            let retrieved_at = timestamp(&at)?;
            // Payload storage deduplicates retrieval-only changes; materialize this run's timestamp.
            let mut payload: serde_json::Value = serde_json::from_str(&payload)?;
            payload["retrieved_at"] = serde_json::to_value(retrieved_at)?;
            Ok(Evidence {
                retrieval: ObservationRetrieval {
                    observation_id,
                    kind: kind.into(),
                    fingerprint,
                    retrieved_at,
                },
                value: serde_json::from_value(payload)?,
            })
        })
        .collect()
    }
}
impl SelectionRepository for SqliteRepository {
    fn financial_runs(&self, p: &ProviderIdentity) -> Result<Vec<FinancialRunEvidence>> {
        validate_identity(p)?;
        let mut stmt=self.connection.prepare("SELECT id,query,operation,status,filings_cursor,facts_cursor,error,started_at,finished_at FROM runs WHERE provider_id=? ORDER BY id")?;
        let rows = stmt.query_map([fingerprint(p)?], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })?;
        rows.map(|row| {
            let (id, q, operation, s, filings_cursor, facts_cursor, error, start, end) = row?;
            Ok(FinancialRunEvidence {
                context: self.selection_context("financial", id, &start, end.as_deref())?,
                run: Run {
                    id,
                    provider: p.clone(),
                    query: serde_json::from_str(&q)?,
                    operation,
                    status: status(&s)?,
                    filings_cursor,
                    facts_cursor,
                    error,
                },
                facts: self.selection_observations(id, "fact", false)?,
                filings: self.selection_observations(id, "filing", false)?,
            })
        })
        .collect()
    }
    fn market_runs(&self, p: &ProviderIdentity) -> Result<Vec<MarketRunEvidence>> {
        validate_identity(p)?;
        let mut stmt=self.connection.prepare("SELECT id,query,status,cursor,coverage,error,started_at,finished_at FROM market_runs WHERE provider_id=? ORDER BY id")?;
        let rows = stmt.query_map([fingerprint(p)?], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, Option<String>>(7)?,
            ))
        })?;
        rows.map(|row| {
            let (id, q, s, cursor, coverage, error, start, end) = row?;
            Ok(MarketRunEvidence {
                context: self.selection_context("market", id, &start, end.as_deref())?,
                run: MarketRun {
                    id,
                    provider: p.clone(),
                    query: serde_json::from_str::<PriceQuery>(&q)?,
                    status: status(&s)?,
                    cursor,
                    coverage: coverage.map(|v| serde_json::from_str(&v)).transpose()?,
                    error,
                },
                prices: self.selection_observations(id, "price", true)?,
            })
        })
        .collect()
    }
}
