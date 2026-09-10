use super::*;
use crate::{AppError, ErrorKind, Result};
/// Pure expected-revision tab transition. Closing preserves the historical view receipt.
pub fn workspace_transition(
    state: &WorkspaceState,
    expected: u64,
    mutation: &WorkspaceMutation,
    limits: &ConversationLimits,
) -> Result<WorkspaceState> {
    limits.validate()?;
    if state.revision != expected {
        return Err(AppError::new(
            ErrorKind::Conflict,
            "workspace revision is stale",
            false,
        ));
    }
    if state.view_ids.len() > limits.open_views {
        return Err(domain::resource_limit());
    }
    crate::agent_contract::check_serialized_size(state, limits.page_bytes)?;
    let mut next = state.clone();
    match mutation {
        WorkspaceMutation::Select { view_id } => {
            validate_id(view_id)?;
            if !next.view_ids.contains(view_id) {
                return Err(domain::invalid("selected view is not open"));
            }
            next.selected_view_id = Some(view_id.clone());
        }
        WorkspaceMutation::Reorder { view_ids } => {
            if view_ids.len() != next.view_ids.len()
                || view_ids
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    != view_ids.len()
                || view_ids.iter().any(|id| !next.view_ids.contains(id))
            {
                return Err(domain::invalid(
                    "reorder must contain every open view exactly once",
                ));
            }
            next.view_ids = view_ids.clone();
        }
        WorkspaceMutation::Close { view_id } => {
            validate_id(view_id)?;
            let index = next
                .view_ids
                .iter()
                .position(|id| id == view_id)
                .ok_or_else(|| domain::invalid("closed view is not open"))?;
            next.view_ids.remove(index);
            if next.selected_view_id.as_ref() == Some(view_id) {
                next.selected_view_id = next
                    .view_ids
                    .get(index.min(next.view_ids.len().saturating_sub(1)))
                    .cloned();
            }
        }
    }
    next.revision = next
        .revision
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or_else(domain::resource_limit)?;
    crate::agent_contract::check_serialized_size(&next, limits.page_bytes)?;
    Ok(next)
}
