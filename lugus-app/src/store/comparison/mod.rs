use super::{SqliteApplicationStore, json, limit, storage};
use crate::{comparison::*, *};
use rusqlite::{OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
mod read;
mod write;
fn scoped() -> AppError {
    AppError::new(
        ErrorKind::ScopeMismatch,
        "Comparison reference is not owned by this workspace and repository",
        false,
    )
}
fn conflict() -> AppError {
    AppError::new(
        ErrorKind::Conflict,
        "Comparison request or execution ownership conflicts",
        false,
    )
}
fn encode<T: Serialize>(s: &SqliteApplicationStore, t: &T) -> Result<String> {
    json(t, s.limits.max_bytes_per_fetch)
}
impl SqliteApplicationStore {
    fn comparison_header<T: DeserializeOwned>(
        &self,
        table: &str,
        s: &Scope,
        id: &str,
    ) -> Result<T> {
        s.validate()?;
        crate::conversations::validate_id(id)?;
        let repo = self.evidence.repository_identity()?;
        let sql = format!(
            "SELECT length(CAST(payload AS BLOB)) FROM {table} WHERE id=?1 AND workspace=?2 AND repository=?3"
        );
        let size: Option<i64> = self
            .connection
            .query_row(&sql, params![id, s.workspace_id, repo], |r| r.get(0))
            .optional()
            .map_err(storage)?;
        if size.ok_or_else(scoped)? as u64 > self.limits.max_output_bytes as u64 {
            return Err(limit());
        }
        let sql =
            format!("SELECT payload FROM {table} WHERE id=?1 AND workspace=?2 AND repository=?3");
        let raw: String = self
            .connection
            .query_row(&sql, params![id, s.workspace_id, repo], |r| r.get(0))
            .map_err(storage)?;
        serde_json::from_str(&raw).map_err(storage)
    }
    fn comparison_page<T: DeserializeOwned + Serialize>(
        &self,
        package: &str,
        section: &str,
        p: PageRequest,
    ) -> Result<ComparisonPage<T>> {
        if p.limit == 0 || p.limit > self.limits.max_read_page_items || p.offset > i64::MAX as usize
        {
            return Err(limit());
        }
        let mut q=self.connection.prepare("SELECT ordinal,length(CAST(payload AS BLOB)) FROM comparison_entries WHERE package=?1 AND section=?2 AND ordinal>=?3 ORDER BY ordinal LIMIT ?4").map_err(storage)?;
        let slots = q
            .query_map(
                params![
                    package,
                    section,
                    p.offset as i64,
                    p.limit.saturating_add(1).min(i64::MAX as usize) as i64
                ],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .map_err(storage)?;
        let mut items = vec![];
        let mut used = 128usize;
        let mut next = None;
        let cap = self
            .limits
            .max_read_page_bytes
            .min(self.limits.max_output_bytes);
        for slot in slots {
            let (ordinal, size) = slot.map_err(storage)?;
            if items.len() == p.limit {
                next = Some(ordinal as usize);
                break;
            }
            if size < 0 || used.saturating_add(size as usize) > cap {
                if items.is_empty() {
                    return Err(limit());
                }
                next = Some(ordinal as usize);
                break;
            }
            let raw:String=self.connection.query_row("SELECT payload FROM comparison_entries WHERE package=?1 AND section=?2 AND ordinal=?3",params![package,section,ordinal],|r|r.get(0)).map_err(storage)?;
            used = used.saturating_add(raw.len() + 1);
            items.push(serde_json::from_str(&raw).map_err(storage)?);
        }
        let result = ComparisonPage {
            items,
            next_offset: next,
        };
        crate::agent_contract::check_serialized_size(&result, cap)?;
        Ok(result)
    }
    fn save_comparison_job(&self, j: &ComparisonJob) -> Result<()> {
        let state = serde_json::to_value(j.state)
            .map_err(storage)?
            .as_str()
            .unwrap()
            .to_owned();
        self.connection
            .execute(
                "UPDATE comparison_jobs SET state=?2,payload=?3 WHERE id=?1",
                params![j.id, state, encode(self, j)?],
            )
            .map_err(storage)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unproven_company_dependencies_atomically() {
        let d = tempfile::tempdir().unwrap();
        let mut s = SqliteApplicationStore::open(
            d.path().join("app"),
            Box::new(
                lugus_financial::storage::SqliteRepository::open(d.path().join("fin")).unwrap(),
            ),
            Limits::default(),
            Box::new(SystemClock),
            Box::new(RandomIds::new().unwrap()),
        )
        .unwrap();
        let scope = Scope {
            workspace_id: "w".into(),
            request_id: "r".into(),
            run_id: None,
        };
        let req:ComparisonRequest=serde_json::from_value(serde_json::json!({"request_id":"r","subjects":[{"text":"AAPL","exchange":null},{"text":"MSFT","exchange":null}],"period_end":"2024-12-31","years":3,"revenue_basis":"revenues"})).unwrap();
        let provider = ProviderIdentity {
            instance_id: "sec".into(),
            plugin_id: "sec-edgar".into(),
            plugin_version: "0.3.0".into(),
        };
        let lease = s.acquire(&scope).unwrap();
        let job = s
            .begin(
                &scope,
                &req,
                &lease,
                &CapturedProviders {
                    facts: provider.clone(),
                    resolution: provider,
                },
            )
            .unwrap();
        let company = |value: &str| ResolvedCompany {
            company: lugus_financial::domain::CompanyId {
                namespace: "sec:cik".into(),
                value: value.into(),
            },
            name: value.into(),
            resolution_dataset_id: "missing".into(),
        };
        let prepared = PreparedComparison {
            companies: [company("1"), company("2")],
            dependencies: vec![],
            rows: vec![],
            sources: vec![],
            entries: vec![],
            issues: vec![],
            partial: true,
        };
        assert!(s.publish(&scope, &job.id, &prepared).is_err());
        assert_eq!(
            s.job(&scope, &job.id).unwrap().state,
            ComparisonState::Running
        );
        assert!(
            s.comparison_list(
                &scope,
                PageRequest {
                    offset: 0,
                    limit: 10
                }
            )
            .unwrap()
            .items
            .is_empty()
        );
    }
}
