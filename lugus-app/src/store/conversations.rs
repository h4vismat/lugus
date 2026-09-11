use super::{error, json, limit, storage};
use crate::{
    AppError, ApplicationStore, ErrorKind, PageRequest, Result, Scope, SqliteApplicationStore,
    conversations::*,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::path::Path;
mod journal;
mod preparation;
mod reads;
mod runs;
mod schema;
mod selection;
mod workspace;
pub(super) use schema::{configured_limits, migrate, migrate_preparations, migrate_recency};
pub(super) use workspace::attach_view;
fn conflict() -> AppError {
    error(
        ErrorKind::Conflict,
        "conversation state conflicts with this operation",
    )
}
fn check_size(size: i64, max: usize) -> Result<()> {
    if size < 0 || size as u64 > max as u64 {
        Err(limit())
    } else {
        Ok(())
    }
}
fn bound<T: Serialize>(value: &T, max: usize) -> Result<()> {
    crate::agent_contract::check_serialized_size(value, max)
}
fn record_cap(s: &SqliteApplicationStore) -> usize {
    s.conversation_limits
        .page_bytes
        .min(s.limits.max_output_bytes)
        .saturating_sub(128)
}
fn authorize(tx: &Connection, id: &str, repository: &str) -> Result<()> {
    validate_id(id)?;
    let matched: Option<bool> = tx
        .query_row(
            "SELECT repository=?2 FROM conversations WHERE id=?1",
            params![id, repository],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    match matched {
        Some(true) => Ok(()),
        Some(false) => Err(error(
            ErrorKind::ScopeMismatch,
            "conversation belongs to another repository",
        )),
        None => Err(error(ErrorKind::MissingData, "conversation does not exist")),
    }
}
fn decode_one<T: DeserializeOwned>(tx: &Connection, sql: &str, id: &str, max: usize) -> Result<T> {
    let metadata = format!("SELECT length(CAST(payload AS BLOB)) FROM ({sql})");
    let size: Option<i64> = tx
        .query_row(&metadata, [id], |r| r.get(0))
        .optional()
        .map_err(storage)?;
    check_size(
        size.ok_or_else(|| error(ErrorKind::MissingData, "conversation record does not exist"))?,
        max,
    )?;
    let payload: String = tx.query_row(sql, [id], |r| r.get(0)).map_err(storage)?;
    serde_json::from_str(&payload).map_err(storage)
}
fn run_read(tx: &Connection, conversation: &str, id: &str, max: usize) -> Result<RunRecord> {
    authorize_run(tx, conversation, id)?;
    decode_one(
        tx,
        "SELECT payload FROM conversation_runs WHERE id=?1",
        id,
        max,
    )
}
fn authorize_run(tx: &Connection, conversation: &str, id: &str) -> Result<()> {
    validate_id(id)?;
    let owns: Option<bool> = tx
        .query_row(
            "SELECT conversation_id=?2 FROM conversation_runs WHERE id=?1",
            params![id, conversation],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    match owns {
        Some(true) => Ok(()),
        Some(false) => Err(error(
            ErrorKind::ScopeMismatch,
            "run belongs to another conversation",
        )),
        None => Err(error(ErrorKind::MissingData, "run does not exist")),
    }
}
fn fence(tx: &Connection, key: &Path, epoch: &ExecutionEpoch) -> Result<()> {
    if epoch.store_key() != key {
        return Err(error(
            ErrorKind::ScopeMismatch,
            "execution lease belongs to another store",
        ));
    }
    let current: i64 = tx
        .query_row(
            "SELECT epoch FROM conversation_config WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(storage)?;
    if current < 1 || current as u64 != epoch.generation() {
        return Err(conflict());
    }
    Ok(())
}
fn active(
    tx: &Connection,
    key: &Path,
    attempt: &RunAttempt,
    repository: &str,
    max: usize,
) -> Result<RunRecord> {
    fence(tx, key, &attempt.epoch)?;
    authorize(tx, &attempt.conversation_id, repository)?;
    let run = run_read(tx, &attempt.conversation_id, &attempt.run_id, max)?;
    if run.epoch != attempt.epoch.generation() || run.status != RunStatus::Running {
        return Err(conflict());
    }
    Ok(run)
}
impl ConversationStore for SqliteApplicationStore {
    fn conversation_limits(&self) -> &ConversationLimits {
        &self.conversation_limits
    }
    fn execution_store_key(&self) -> &Path {
        &self.store_key
    }
    fn create_conversation(&mut self, r: &str, t: &str) -> Result<Conversation> {
        self.conversation_create(r, t)
    }
    fn conversation(&self, id: &str) -> Result<Conversation> {
        self.conversation_read(id)
    }
    fn conversations(&self, p: PageRequest) -> Result<ConversationPage<Conversation>> {
        self.conversation_list(p)
    }
    fn recent_conversations(&self, p: PageRequest) -> Result<ConversationPage<Conversation>> {
        self.conversation_recent_list(p)
    }
    fn workspace(&self, id: &str) -> Result<WorkspaceState> {
        self.workspace_read(id)
    }
    fn mutate_workspace(
        &mut self,
        id: &str,
        rev: u64,
        m: &WorkspaceMutation,
    ) -> Result<WorkspaceState> {
        self.workspace_mutate(id, rev, m)
    }
    fn messages(&self, id: &str, p: PageRequest) -> Result<ConversationPage<Message>> {
        self.conversation_page(id, None, "conversation_messages", "conversation_id", p)
    }
    fn run(&self, id: &str, run: &str) -> Result<RunRecord> {
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        authorize(&tx, id, &self.evidence.repository_identity()?)?;
        let result = run_read(&tx, id, run, record_cap(self))?;
        tx.commit().map_err(storage)?;
        Ok(result)
    }
    fn save_preparation(&mut self, attempt: &RunAttempt, serialized: &str) -> Result<()> {
        self.preparation_save(attempt, serialized)
    }
    fn preparation(&self, conversation_id: &str, run_id: &str) -> Result<Option<String>> {
        self.preparation_read(conversation_id, Some(run_id))
    }
    fn latest_preparation(&self, conversation_id: &str) -> Result<Option<String>> {
        self.preparation_read(conversation_id, None)
    }
    fn runs(&self, id: &str, p: PageRequest) -> Result<ConversationPage<RunRecord>> {
        self.conversation_page(id, None, "conversation_runs", "conversation_id", p)
    }
    fn activity(
        &self,
        id: &str,
        run: &str,
        p: PageRequest,
    ) -> Result<ConversationPage<ActivityRecord>> {
        self.conversation_page(id, Some(run), "conversation_activity", "run_id", p)
    }
    fn tool_records(
        &self,
        id: &str,
        run: &str,
        p: PageRequest,
    ) -> Result<ConversationPage<ToolRecord>> {
        self.conversation_page(id, Some(run), "conversation_tools", "run_id", p)
    }
    fn activate(&mut self, l: &LocalExecutionLease) -> Result<ExecutionEpoch> {
        self.conversation_activate(l)
    }
    fn lookup_request(&self, r: &SendMessageRequest) -> Result<Option<RunRecord>> {
        self.conversation_lookup(r)
    }
    fn admit(&mut self, e: &ExecutionEpoch, r: &SendMessageRequest) -> Result<RunRecord> {
        self.conversation_admit(e, r)
    }
    fn start(&mut self, e: &ExecutionEpoch, c: &str, r: &str) -> Result<RunAttempt> {
        self.conversation_start(e, c, r)
    }
    fn fail_admission(
        &mut self,
        e: &ExecutionEpoch,
        c: &str,
        r: &str,
        error: &AppError,
    ) -> Result<RunRecord> {
        self.conversation_fail_admission(e, c, r, error)
    }
    fn append_activity(&mut self, a: &RunAttempt, k: &str, d: &str) -> Result<ActivityRecord> {
        self.activity_append(a, k, d)
    }
    fn begin_tool(&mut self, a: &RunAttempt, i: &ToolIntent) -> Result<BeginTool> {
        self.tool_begin(a, i)
    }
    fn finish_tool(&mut self, a: &RunAttempt, c: &str, o: &ToolOutcome) -> Result<ToolRecord> {
        self.tool_finish(a, c, o)
    }
    fn finish_run(&mut self, a: &RunAttempt, c: &RunCompletion) -> Result<RunRecord> {
        self.conversation_finish(a, c)
    }
}
