//! Bounded offline reads of immutable binding records and history.
use super::binding_history::event;
use super::*;
use rusqlite::{Connection, params};
use serde::de::DeserializeOwned;
impl SqliteApplicationStore {
    fn page_bounds(&self, page: PageRequest) -> Result<()> {
        if page.limit == 0
            || page.limit > self.limits.max_read_page_items
            || page.offset > i64::MAX as usize
        {
            return Err(limit());
        }
        Ok(())
    }
    pub(super) fn binding_page(&self, scope: &Scope, page: PageRequest) -> Result<BindingPage> {
        scope.validate()?;
        self.page_bounds(page)?;
        let repository = self.evidence.repository_identity()?;
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        let sql = "SELECT a.payload FROM binding_index b JOIN app_records a ON a.id=b.binding_id WHERE b.workspace=?1 AND b.repository=?2 ORDER BY b.sequence LIMIT ?3 OFFSET ?4";
        let bytes:i64=tx.query_row("SELECT coalesce(sum(record_size+event_size),0) FROM (SELECT length(CAST(a.payload AS BLOB)) record_size,(SELECT length(CAST(h.payload AS BLOB)) FROM binding_history h WHERE h.binding_id=b.binding_id ORDER BY h.sequence DESC LIMIT 1) event_size FROM binding_index b JOIN app_records a ON a.id=b.binding_id WHERE b.workspace=?1 AND b.repository=?2 ORDER BY b.sequence LIMIT ?3 OFFSET ?4)",params![scope.workspace_id,repository,page.limit as i64,page.offset as i64],|r|r.get(0)).map_err(storage)?;
        if bytes < 0 || bytes as u64 > self.limits.max_read_page_bytes as u64 {
            return Err(limit());
        }
        let records: Vec<BindingRecord> = bounded_rows(
            &tx,
            sql,
            params![
                scope.workspace_id,
                repository,
                page.limit as i64,
                page.offset as i64
            ],
            self.limits.max_read_page_bytes,
        )?;
        let bindings = records
            .into_iter()
            .map(|record| {
                Ok(BindingView {
                    status: event(&tx, &record.id, self.limits.max_read_page_bytes)?.status,
                    record,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let end = page.offset.checked_add(bindings.len()).ok_or_else(limit)?;
        let more:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM binding_index WHERE workspace=?1 AND repository=?2 ORDER BY sequence LIMIT 1 OFFSET ?3)",params![scope.workspace_id,repository,end as i64],|r|r.get(0)).map_err(storage)?;
        let result = BindingPage {
            bindings,
            next_offset: more.then_some(end),
        };
        json(
            &result,
            self.limits
                .max_read_page_bytes
                .min(self.limits.max_output_bytes),
        )?;
        tx.commit().map_err(storage)?;
        Ok(result)
    }
    pub(super) fn history(
        &self,
        scope: &Scope,
        id: &str,
        page: PageRequest,
    ) -> Result<BindingHistoryPage> {
        self.binding_view(scope, id)?;
        self.page_bounds(page)?;
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        let events = bounded_rows(
            &tx,
            "SELECT payload FROM binding_history WHERE binding_id=?1 ORDER BY sequence LIMIT ?2 OFFSET ?3",
            params![id, page.limit as i64, page.offset as i64],
            self.limits.max_read_page_bytes,
        )?;
        let end = page.offset.checked_add(events.len()).ok_or_else(limit)?;
        let more:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM binding_history WHERE binding_id=?1 ORDER BY sequence LIMIT 1 OFFSET ?2)",params![id,end as i64],|r|r.get(0)).map_err(storage)?;
        let result = BindingHistoryPage {
            events,
            next_offset: more.then_some(end),
        };
        json(
            &result,
            self.limits
                .max_read_page_bytes
                .min(self.limits.max_output_bytes),
        )?;
        tx.commit().map_err(storage)?;
        Ok(result)
    }
}
fn bounded_rows<T: DeserializeOwned>(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params + Clone,
    max: usize,
) -> Result<Vec<T>> {
    let size: i64 = conn
        .query_row(
            &format!("SELECT coalesce(sum(length(CAST(payload AS BLOB))),0) FROM ({sql})"),
            params.clone(),
            |r| r.get(0),
        )
        .map_err(storage)?;
    if size < 0 || size as u64 > max as u64 {
        return Err(limit());
    }
    let mut stmt = conn.prepare(sql).map_err(storage)?;
    stmt.query_map(params, |r| r.get::<_, String>(0))
        .map_err(storage)?
        .map(|r| serde_json::from_str(&r.map_err(storage)?).map_err(storage))
        .collect()
}
