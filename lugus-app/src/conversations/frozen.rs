use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
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
    pub fn from_portfolio(
        snapshot: &crate::portfolio::PortfolioSnapshot,
        limits: &ConversationLimits,
    ) -> Result<Self> {
        validate_id(&snapshot.id)?;
        Self::freeze(
            SelectedReference::Portfolio {
                id: snapshot.id.clone(),
            },
            snapshot,
            limits,
        )
    }

    /// Freeze only a passage obtained from a scoped trusted store read.
    pub fn from_passage(
        passage: &crate::passages::Passage,
        limits: &ConversationLimits,
    ) -> Result<Self> {
        limits.validate()?;
        validate_passage(passage)?;
        Self::freeze(
            SelectedReference::Passage {
                id: passage.id.clone(),
            },
            passage,
            limits,
        )
    }

    /// Caller must use the application's scoped read of offset zero. No latest query is performed.
    pub fn from_dataset(page: &DatasetPage, limits: &ConversationLimits) -> Result<Self> {
        limits.validate()?;
        let coverage = dataset_coverage(page, limits)?;
        #[derive(Serialize)]
        struct Payload<'a> {
            page: &'a DatasetPage,
            coverage: DatasetSelectionCoverage,
        }
        let payload = Payload { page, coverage };
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
        validate_view_ids(&view.id, &view.workspace_id, &view.dataset_id)?;
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
        validate_binding(binding)?;
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
        limits.validate()?;
        self.reference.validate()?;
        crate::agent_contract::check_serialized_size(self, limits.selected_bytes)?;
        if self.checksum != checksum(&self.serialized) {
            return Err(invalid("frozen reference integrity check failed"));
        }
        let payload_id = match &self.reference {
            SelectedReference::Portfolio { .. } => {
                let snapshot: crate::portfolio::PortfolioSnapshot =
                    decode_frozen(&self.serialized, limits.selected_bytes)?;
                snapshot.id
            }

            SelectedReference::Passage { .. } => {
                let payload: crate::passages::Passage =
                    decode_frozen(&self.serialized, limits.selected_bytes)?;
                validate_passage(&payload)?;
                payload.id
            }
            SelectedReference::Dataset { .. } => {
                let payload: FrozenDataset =
                    decode_frozen(&self.serialized, limits.selected_bytes)?;
                if payload.coverage != dataset_coverage(&payload.page, limits)? {
                    return Err(invalid(
                        "frozen dataset coverage does not match its first page",
                    ));
                }
                payload.page.header.id
            }
            SelectedReference::View { .. } => {
                let payload: FrozenView = decode_frozen(&self.serialized, limits.selected_bytes)?;
                validate_view_ids(&payload.id, &payload.workspace_id, &payload.dataset_id)?;
                payload.id
            }
            SelectedReference::Binding { .. } => {
                let payload: BindingRecord =
                    decode_frozen(&self.serialized, limits.selected_bytes)?;
                validate_binding(&payload)?;
                payload.id
            }
        };
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

/// Shared by construction and restoration so changed limits cannot weaken a frozen selection.
fn dataset_coverage(
    page: &DatasetPage,
    limits: &ConversationLimits,
) -> Result<DatasetSelectionCoverage> {
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
    Ok(DatasetSelectionCoverage {
        offset: 0,
        returned_rows,
        total_rows,
        next_offset: page.next_offset,
        complete: returned_rows == total_rows,
    })
}

fn validate_view_ids(id: &str, workspace_id: &str, dataset_id: &str) -> Result<()> {
    for id in [id, workspace_id, dataset_id] {
        validate_id(id)?;
    }
    Ok(())
}

fn validate_binding(binding: &BindingRecord) -> Result<()> {
    validate_id(&binding.id)?;
    validate_id(&binding.repository_id)?;
    binding.scope.validate()
}

fn decode_frozen<T: DeserializeOwned + Serialize>(serialized: &str, max_bytes: usize) -> Result<T> {
    // The complete FrozenReference envelope is preflighted before this decoder is called.
    // Keep a local input check as well, and preflight reserialization before allocating a Value.
    if serialized.len() > max_bytes {
        return Err(resource_limit());
    }
    let payload: T = serde_json::from_str(serialized)
        .map_err(|_| invalid("frozen reference payload does not match its type"))?;
    crate::agent_contract::check_serialized_size(&payload, max_bytes)?;
    let original: serde_json::Value = serde_json::from_str(serialized)
        .map_err(|_| invalid("frozen reference payload is invalid JSON"))?;
    let restored = serde_json::to_value(&payload)
        .map_err(|_| invalid("frozen reference payload could not be serialized"))?;
    // Legacy DTOs may discard unknown fields or normalize/default values. Frozen records
    // require exactly the constructor's typed shape, recursively, without changing those DTOs.
    // Object key order/whitespace are immaterial; intentional Value fields remain opaque data.
    if original != restored {
        return Err(invalid(
            "frozen reference payload does not preserve its typed shape",
        ));
    }
    Ok(payload)
}

// Restoration checks the self-contained immutable contract without looking up current evidence.
fn validate_passage(p: &crate::passages::Passage) -> Result<()> {
    use crate::passages::{MappingKind, TextLimits, text_checksum};
    let h = &p.representation;
    let cap = TextLimits::default();
    p.scope.validate()?;
    for id in [
        &p.id,
        &h.id,
        &h.workspace_id,
        &h.repository_id,
        &h.dataset_id,
        &h.extractor.format,
        &h.extractor.policy,
        &h.extractor.parser,
        &h.extractor.decoder,
        &h.decoder,
        &h.document.provider.instance_id,
        &h.document.provider.plugin_id,
        &h.document.provider.plugin_version,
    ] {
        validate_id(id)?;
    }
    let digest = |s: &str| {
        s.len() == 64
            && s.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    };
    if h.workspace_id != p.scope.workspace_id
        || !digest(&h.text_checksum)
        || !digest(&h.document.checksum)
        || p.quote_checksum != text_checksum(&p.quote)
        || p.quote.is_empty()
        || p.quote.len() > cap.max_passage_bytes
        || p.end.checked_sub(p.start) != Some(p.quote.len())
        || p.end > h.text_bytes
        || h.text_bytes > cap.max_text_bytes
        || h.source_node_count == 0
        || h.source_node_count > cap.max_nodes
        || h.mapping_count > cap.max_mappings
        || p.mappings.is_empty()
        || p.mappings.len() > h.mapping_count
        || h.document.source_url.is_empty()
        || h.document.media_type.is_empty()
    {
        return Err(invalid("frozen passage metadata is inconsistent"));
    }
    let mut end = p.start;
    let mut previous: Option<&crate::passages::SourceMapping> = None;
    let mut source_backed = false;
    for m in &p.mappings {
        let overlap = previous.is_some_and(|prior| {
            prior.kind == MappingKind::Normalized
                && m.kind == MappingKind::Normalized
                && prior.start == m.start
                && prior.end == m.end
        });
        if m.start < p.start || m.end > p.end || m.start >= m.end || (m.start != end && !overlap) {
            return Err(invalid("frozen passage mappings do not cover its quote"));
        }
        let selected = p
            .quote
            .get(m.start - p.start..m.end - p.start)
            .ok_or_else(|| invalid("frozen passage mapping splits UTF-8"))?;
        match (&m.kind, &m.source) {
            (MappingKind::Synthetic, None) if selected.chars().all(char::is_whitespace) => (),
            (MappingKind::Exact | MappingKind::Normalized, Some(source)) => {
                // Node IDs are opaque u32 identities; max_nodes bounds their count above.
                if source.start >= source.end
                    || source.end > cap.max_source_bytes
                    || (m.kind == MappingKind::Exact
                        && source.end - source.start != m.end - m.start)
                    || (m.kind == MappingKind::Normalized && selected != " ")
                {
                    return Err(invalid("frozen passage source interval is inconsistent"));
                }
                source_backed = true;
            }
            _ => return Err(invalid("frozen passage mapping kind is inconsistent")),
        }
        end = m.end;
        previous = Some(m);
    }
    if end != p.end || !source_backed {
        return Err(invalid("frozen passage requires complete source mappings"));
    }
    Ok(())
}
