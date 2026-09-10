use super::*;
impl SqliteApplicationStore {
    pub(super) fn workspace_read(&self, id: &str) -> Result<WorkspaceState> {
        let tx = self.connection.unchecked_transaction().map_err(storage)?;
        authorize(&tx, id, &self.evidence.repository_identity()?)?;
        let state = decode_one(
            &tx,
            "SELECT payload FROM conversation_workspaces WHERE conversation_id=?1",
            id,
            record_cap(self),
        )?;
        tx.commit().map_err(storage)?;
        Ok(state)
    }
    pub(super) fn workspace_mutate(
        &mut self,
        id: &str,
        revision: u64,
        mutation: &WorkspaceMutation,
    ) -> Result<WorkspaceState> {
        let max = record_cap(self);
        bound(mutation, max)?;
        let repository = self.evidence.repository_identity()?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        authorize(&tx, id, &repository)?;
        let state = decode_one(
            &tx,
            "SELECT payload FROM conversation_workspaces WHERE conversation_id=?1",
            id,
            max,
        )?;
        let next = workspace_transition(&state, revision, mutation, &self.conversation_limits)?;
        save_layout(&tx, &next, max)?;
        tx.commit().map_err(storage)?;
        Ok(next)
    }
}
pub(super) fn save_layout(tx: &Connection, state: &WorkspaceState, max: usize) -> Result<()> {
    let payload = json(state, max)?;
    tx.execute(
        "UPDATE conversation_workspaces SET revision=?2,payload=?3 WHERE conversation_id=?1",
        params![state.conversation_id, state.revision as i64, payload],
    )
    .map_err(storage)?;
    Ok(())
}
pub(in crate::store) fn attach_view(
    tx: &Connection,
    workspace: &str,
    repository: &str,
    view: &str,
    limits: &ConversationLimits,
    max: usize,
) -> Result<()> {
    let id_size: Option<i64> = tx
        .query_row(
            "SELECT length(CAST(id AS BLOB)) FROM conversations WHERE workspace=?1",
            [workspace],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    let Some(size) = id_size else { return Ok(()) };
    check_size(size, Scope::MAX_ID_BYTES)?;
    let id: String = tx
        .query_row(
            "SELECT id FROM conversations WHERE workspace=?1",
            [workspace],
            |r| r.get(0),
        )
        .map_err(storage)?;
    authorize(tx, &id, repository)?;
    let mut state: WorkspaceState = decode_one(
        tx,
        "SELECT payload FROM conversation_workspaces WHERE conversation_id=?1",
        &id,
        max,
    )?;
    if state.view_ids.len() >= limits.open_views {
        return Err(limit());
    }
    validate_id(view)?;
    state.view_ids.push(view.into());
    if state.selected_view_id.is_none() {
        state.selected_view_id = Some(view.into());
    }
    state.revision = state
        .revision
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or_else(limit)?;
    save_layout(tx, &state, max)
}
