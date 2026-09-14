//! Required references use the existing scoped record reader with stricter conversation budgets.
use super::*;
use crate::{BindingRecord, DatasetHeader, DatasetPage, DatasetRow, ViewReceipt};
impl SqliteApplicationStore {
    pub(super) fn freeze_selected(
        &self,
        scope: &Scope,
        selected: &SelectedReference,
    ) -> Result<FrozenReference> {
        let limits = &self.conversation_limits;
        let max = limits.selected_bytes.min(self.limits.max_output_bytes);
        match selected {
            SelectedReference::Portfolio { id } => {
                let conversation: String = self
                    .connection
                    .query_row(
                        "SELECT id FROM conversations WHERE workspace=?1",
                        [&scope.workspace_id],
                        |row| row.get(0),
                    )
                    .map_err(storage)?;
                let snapshot = crate::portfolio::PortfolioStore::portfolio_read_snapshot(
                    self,
                    &conversation,
                    id,
                )?;
                FrozenReference::from_portfolio(&snapshot, limits)
            }

            SelectedReference::Passage { id } => FrozenReference::from_passage(
                &crate::PassageStore::read_passage(self, scope, id, max)?,
                limits,
            ),
            SelectedReference::View { id } => FrozenReference::from_view(
                &self.read::<ViewReceipt>(scope, id, "view", max)?,
                limits,
            ),
            SelectedReference::Binding { id } => {
                // Preflight under conversation bounds before the existing active-status check.
                self.read::<BindingRecord>(scope, id, "binding", max)?;
                FrozenReference::from_binding(&self.prepare_binding(scope, id)?, limits)
            }
            SelectedReference::Dataset { id } => {
                let max = max.min(limits.page_bytes);
                let header: DatasetHeader = self.read(scope, id, "dataset", max)?;
                let count_limit = limits.page_items.min(self.limits.max_read_page_items);
                let tx = self.connection.unchecked_transaction().map_err(storage)?;
                let (count,bytes):(i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))),0) FROM (SELECT payload FROM dataset_rows WHERE dataset_id=?1 ORDER BY ordinal LIMIT ?2)",params![id,count_limit as i64],|r|Ok((r.get(0)?,r.get(1)?))).map_err(storage)?;
                check_size(bytes, max.min(self.limits.max_read_page_bytes))?;
                let mut page = DatasetPage {
                    next_offset: ((count as u64) < header.row_count as u64)
                        .then_some(count as usize),
                    header,
                    rows: vec![],
                };
                let overhead = json(&page, max)?
                    .len()
                    .checked_add(count.saturating_sub(1).max(0) as usize)
                    .ok_or_else(limit)?;
                if overhead.checked_add(bytes as usize).is_none_or(|n| n > max) {
                    return Err(limit());
                }
                {
                    let mut stmt=tx.prepare("SELECT payload FROM dataset_rows WHERE dataset_id=?1 ORDER BY ordinal LIMIT ?2").map_err(storage)?;
                    page.rows = stmt
                        .query_map(params![id, count_limit as i64], |r| r.get::<_, String>(0))
                        .map_err(storage)?
                        .map(|row| serde_json::from_str(&row.map_err(storage)?).map_err(storage))
                        .collect::<Result<Vec<DatasetRow>>>()?;
                }
                tx.commit().map_err(storage)?;
                FrozenReference::from_dataset(&page, limits)
            }
        }
    }
}
