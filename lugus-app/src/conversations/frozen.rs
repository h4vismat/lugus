use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    ConversationLimits, FrozenReference, SelectedReference,
    domain::{invalid, resource_limit, validate_id},
};
use crate::{BindingRecord, DatasetPage, Result, ViewKind, ViewReceipt};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetSelectionCoverage {
    pub offset: usize,
    pub returned_rows: usize,
    pub total_rows: usize,
    pub next_offset: Option<usize>,
    pub complete: bool,
}

/// An explicit first page, including its immutable header. Document datasets retain
/// their original document observation; this selection does not claim to contain document bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenDataset {
    pub page: DatasetPage,
    pub coverage: DatasetSelectionCoverage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenView {
    pub id: String,
    pub workspace_id: String,
    pub dataset_id: String,
    pub kind: ViewKind,
    pub descriptor_revision: u32,
    pub accepted_at: DateTime<Utc>,
}

impl FrozenReference {
    /// Caller must use the application's scoped read of offset zero. No latest query is performed.
    pub fn from_dataset(page: &DatasetPage, limits: &ConversationLimits) -> Result<Self> {
        limits.validate()?;
        for id in [
            &page.header.id,
            &page.header.workspace_id,
            &page.header.repository_id,
        ] {
            validate_id(id)?;
        }
        if page.rows.len() > limits.page_items {
            return Err(resource_limit());
        }
        let returned_rows = page.rows.len();
        let total_rows = page.header.row_count;
        if returned_rows > total_rows
            || page.next_offset != (returned_rows < total_rows).then_some(returned_rows)
        {
            return Err(invalid(
                "selected dataset must be an explicit consistent first page",
            ));
        }
        crate::agent_contract::check_serialized_size(page, limits.page_bytes)?;
        #[derive(Serialize)]
        struct Payload<'a> {
            page: &'a DatasetPage,
            coverage: DatasetSelectionCoverage,
        }
        let payload = Payload {
            page,
            coverage: DatasetSelectionCoverage {
                offset: 0,
                returned_rows,
                total_rows,
                next_offset: page.next_offset,
                complete: returned_rows == total_rows,
            },
        };
        Self::freeze(
            SelectedReference::Dataset {
                id: page.header.id.clone(),
            },
            &payload,
            limits,
        )
    }

    /// Presentation status is mutable and deliberately excluded from the pinned descriptor.
    pub fn from_view(view: &ViewReceipt, limits: &ConversationLimits) -> Result<Self> {
        limits.validate()?;
        for id in [&view.id, &view.workspace_id, &view.dataset_id] {
            validate_id(id)?;
        }
        let payload = FrozenView {
            id: view.id.clone(),
            workspace_id: view.workspace_id.clone(),
            dataset_id: view.dataset_id.clone(),
            kind: view.kind,
            descriptor_revision: view.descriptor_revision,
            accepted_at: view.accepted_at,
        };
        Self::freeze(
            SelectedReference::View {
                id: view.id.clone(),
            },
            &payload,
            limits,
        )
    }

    /// Accept the immutable record returned by a scoped trusted read, never BindingView's current status.
    pub fn from_binding(binding: &BindingRecord, limits: &ConversationLimits) -> Result<Self> {
        limits.validate()?;
        validate_id(&binding.id)?;
        validate_id(&binding.repository_id)?;
        binding.scope.validate()?;
        Self::freeze(
            SelectedReference::Binding {
                id: binding.id.clone(),
            },
            binding,
            limits,
        )
    }

    fn freeze(
        reference: SelectedReference,
        payload: &impl Serialize,
        limits: &ConversationLimits,
    ) -> Result<Self> {
        let serialized = super::context::bounded_json(payload, limits.selected_bytes)?;
        let checksum = checksum(&serialized);
        let result = Self {
            reference,
            serialized,
            checksum,
        };
        crate::agent_contract::check_serialized_size(&result, limits.selected_bytes)?;
        Ok(result)
    }

    pub fn validate(&self, limits: &ConversationLimits) -> Result<()> {
        self.reference.validate()?;
        crate::agent_contract::check_serialized_size(self, limits.selected_bytes)?;
        if self.checksum != checksum(&self.serialized) {
            return Err(invalid("frozen reference integrity check failed"));
        }
        let payload_id = match &self.reference {
            SelectedReference::Dataset { .. } => {
                serde_json::from_str::<FrozenDataset>(&self.serialized)
                    .map(|payload| payload.page.header.id)
            }
            SelectedReference::View { .. } => {
                serde_json::from_str::<FrozenView>(&self.serialized).map(|payload| payload.id)
            }
            SelectedReference::Binding { .. } => {
                serde_json::from_str::<BindingRecord>(&self.serialized).map(|payload| payload.id)
            }
        }
        .map_err(|_| invalid("frozen reference payload does not match its type"))?;
        if payload_id != self.reference.id() {
            return Err(invalid(
                "frozen reference identity does not match its payload",
            ));
        }
        Ok(())
    }
}

fn checksum(serialized: &str) -> String {
    format!("{:x}", Sha256::digest(serialized.as_bytes()))
}
