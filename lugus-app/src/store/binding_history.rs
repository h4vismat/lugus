//! Additive schema and serialized append-only binding state transitions.
use super::*;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
pub(super) fn migrate(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch("CREATE TABLE binding_index(sequence INTEGER PRIMARY KEY AUTOINCREMENT,binding_id TEXT NOT NULL UNIQUE REFERENCES app_records(id),workspace TEXT NOT NULL,repository TEXT NOT NULL);
 CREATE INDEX binding_scope ON binding_index(workspace,repository,sequence);
 CREATE TABLE binding_history(sequence INTEGER PRIMARY KEY AUTOINCREMENT,binding_id TEXT NOT NULL REFERENCES app_records(id),payload TEXT NOT NULL);
 CREATE INDEX binding_events ON binding_history(binding_id,sequence);
 CREATE TABLE binding_requests(workspace TEXT NOT NULL,repository TEXT NOT NULL,request TEXT NOT NULL,input TEXT NOT NULL,binding_id TEXT NOT NULL REFERENCES app_records(id),PRIMARY KEY(workspace,repository,request));
 CREATE TRIGGER binding_record_update BEFORE UPDATE ON app_records WHEN OLD.category='binding' BEGIN SELECT RAISE(ABORT,'immutable binding'); END;
 CREATE TRIGGER binding_record_delete BEFORE DELETE ON app_records WHEN OLD.category='binding' BEGIN SELECT RAISE(ABORT,'immutable binding'); END;
 PRAGMA user_version=2;").map_err(storage)?;
    for table in ["binding_index", "binding_history", "binding_requests"] {
        for operation in ["UPDATE", "DELETE"] {
            tx.execute_batch(&format!("CREATE TRIGGER {table}_{operation} BEFORE {operation} ON {table} BEGIN SELECT RAISE(ABORT,'immutable binding history'); END;")).map_err(storage)?;
        }
    }
    Ok(())
}
pub(super) fn conflict() -> AppError {
    error(
        ErrorKind::Conflict,
        "binding request conflicts with immutable history",
    )
}
pub(super) fn dedupe(
    conn: &Connection,
    scope: &Scope,
    repository: &str,
    input: &str,
    max: usize,
) -> Result<Option<String>> {
    let existing:Option<(bool,i64)>=conn.query_row("SELECT input=?4,length(CAST(binding_id AS BLOB)) FROM binding_requests WHERE workspace=?1 AND repository=?2 AND request=?3",params![scope.workspace_id,repository,scope.request_id,input],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(storage)?;
    if let Some((same, size)) = existing {
        if !same {
            return Err(conflict());
        }
        if size < 0 || size as u64 > max.min(Scope::MAX_ID_BYTES) as u64 {
            return Err(limit());
        }
        return conn.query_row("SELECT binding_id FROM binding_requests WHERE workspace=?1 AND repository=?2 AND request=?3",params![scope.workspace_id,repository,scope.request_id],|r|r.get(0)).map(Some).map_err(storage);
    }
    Ok(None)
}
pub(super) fn event(conn: &Connection, id: &str, max: usize) -> Result<BindingEvent> {
    let size:i64=conn.query_row("SELECT length(CAST(payload AS BLOB)) FROM binding_history WHERE binding_id=?1 ORDER BY sequence DESC LIMIT 1",[id],|r|r.get(0)).map_err(storage)?;
    if size < 0 || size as u64 > max as u64 {
        return Err(limit());
    }
    let payload:String=conn.query_row("SELECT payload FROM binding_history WHERE binding_id=?1 ORDER BY sequence DESC LIMIT 1",[id],|r|r.get(0)).map_err(storage)?;
    serde_json::from_str(&payload).map_err(storage)
}
pub(super) fn append(
    conn: &Connection,
    scope: &Scope,
    id: &str,
    status: BindingStatus,
    now: chrono::DateTime<chrono::Utc>,
    max: usize,
) -> Result<()> {
    let value = BindingEvent {
        scope: scope.clone(),
        binding_id: id.into(),
        status,
        created_at: now,
    };
    conn.execute(
        "INSERT INTO binding_history(binding_id,payload) VALUES(?1,?2)",
        params![id, json(&value, max)?],
    )
    .map_err(storage)?;
    Ok(())
}
impl SqliteApplicationStore {
    pub(super) fn revoke(
        &mut self,
        scope: &Scope,
        request: &RevokeBindingRequest,
    ) -> Result<BindingView> {
        scope.validate()?;
        let record: BindingRecord = self.read(
            scope,
            &request.binding_id,
            "binding",
            self.limits.max_output_bytes,
        )?;
        // Check the actual returned state before appending a transition, including
        // records created under older admission rules or reopened with lower limits.
        json(
            &BindingView {
                record: record.clone(),
                status: BindingStatus::Revoked,
            },
            self.limits.max_output_bytes,
        )?;
        let input = json(
            &("revoke", &scope.run_id, request),
            self.limits.max_input_bytes,
        )?;
        let now = self.clock.now();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        if dedupe(
            &tx,
            scope,
            &record.repository_id,
            &input,
            self.limits.max_input_bytes,
        )?
        .is_none()
        {
            if event(&tx, &request.binding_id, self.limits.max_output_bytes)?.status
                != BindingStatus::Active
            {
                return Err(conflict());
            }
            append(
                &tx,
                scope,
                &request.binding_id,
                BindingStatus::Revoked,
                now,
                self.limits.max_output_bytes,
            )?;
            tx.execute("INSERT INTO binding_requests(workspace,repository,request,input,binding_id) VALUES(?1,?2,?3,?4,?5)",params![scope.workspace_id,record.repository_id,scope.request_id,input,request.binding_id]).map_err(storage)?;
        }
        tx.commit().map_err(storage)?;
        self.binding_view(scope, &request.binding_id)
    }
}
