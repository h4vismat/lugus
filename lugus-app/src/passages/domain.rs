use crate::{AppError, ErrorKind, Limits, Result, Scope};
use chrono::{DateTime, Utc};
use lugus_financial::storage::DocumentObservation;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextLimits {
    pub max_input_bytes: usize,
    pub max_text_bytes: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_mappings: usize,
    pub max_source_bytes: usize,
    pub max_page_bytes: usize,
    pub max_passage_bytes: usize,
}
impl Default for TextLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 32 * 1024 * 1024,
            max_text_bytes: 16 * 1024 * 1024,
            max_nodes: 200_000,
            max_depth: 256,
            max_mappings: 500_000,
            max_source_bytes: 64 * 1024 * 1024,
            max_page_bytes: 64 * 1024,
            max_passage_bytes: 8 * 1024,
        }
    }
}
impl TextLimits {
    /// Defaults are hard ceilings; deployments may tighten each independent budget.
    pub fn validate(&self) -> Result<()> {
        let cap = Self::default();
        for (value, maximum) in [
            (self.max_input_bytes, cap.max_input_bytes),
            (self.max_text_bytes, cap.max_text_bytes),
            (self.max_nodes, cap.max_nodes),
            (self.max_depth, cap.max_depth),
            (self.max_mappings, cap.max_mappings),
            (self.max_source_bytes, cap.max_source_bytes),
            (self.max_page_bytes, cap.max_page_bytes),
            (self.max_passage_bytes, cap.max_passage_bytes),
        ] {
            if value == 0 || value > maximum {
                return Err(invalid("text limits are outside safe bounds"));
            }
        }
        Ok(())
    }
    pub fn effective(&self, app: &Limits) -> Result<Self> {
        self.validate()?;
        app.validate()?;
        let mut result = self.clone();
        result.max_input_bytes = result.max_input_bytes.min(app.max_document_bytes);
        result.max_page_bytes = result
            .max_page_bytes
            .min(app.max_read_page_bytes)
            .min(app.max_output_bytes);
        result.max_passage_bytes = result.max_passage_bytes.min(app.max_output_bytes);
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractorIdentity {
    pub format: String,
    pub policy: String,
    pub parser: String,
    pub decoder: String,
}
impl ExtractorIdentity {
    pub fn html_v1() -> Self {
        Self {
            format: "html".into(),
            policy: "html-text-v1".into(),
            parser: "html5ever-0.38.0".into(),
            decoder: "encoding_rs-0.8.40".into(),
        }
    }
}

/// Owned preparation result. Persistence splits text and source-node text into chunks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedText {
    pub extractor: ExtractorIdentity,
    pub decoder: String,
    pub text: String,
    pub text_checksum: String,
    pub source_nodes: Vec<SourceNode>,
    pub mappings: Vec<SourceMapping>,
    pub limitations: Vec<String>,
}

/// Immutable metadata only: a small read never needs the full extraction payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextRepresentation {
    pub id: String,
    pub workspace_id: String,
    pub repository_id: String,
    pub dataset_id: String,
    pub document: DocumentObservation,
    pub extractor: ExtractorIdentity,
    pub decoder: String,
    pub text_checksum: String,
    pub text_bytes: usize,
    pub source_node_count: usize,
    pub mapping_count: usize,
    pub limitations: Vec<String>,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextPage {
    pub representation_id: String,
    pub start: usize,
    pub end: usize,
    pub total_bytes: usize,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceNode {
    pub node_id: u32,
    /// Zero-based child ordinals from the parsed document node; not raw HTML offsets.
    pub path: Vec<u32>,
    pub text: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MappingKind {
    Exact,
    Normalized,
    Synthetic,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInterval {
    pub node_id: u32,
    pub start: usize,
    pub end: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMapping {
    pub start: usize,
    pub end: usize,
    pub kind: MappingKind,
    pub source: Option<SourceInterval>,
}
/// A bounded source slice. `start`/`end` address the full decoded source node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceExcerpt {
    pub node_id: u32,
    pub path: Vec<u32>,
    pub start: usize,
    pub end: usize,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePassageRequest {
    pub representation_id: String,
    pub start: usize,
    pub end: usize,
    pub expected_text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Passage {
    pub id: String,
    pub scope: Scope,
    pub representation: TextRepresentation,
    pub start: usize,
    pub end: usize,
    pub quote: String,
    pub quote_checksum: String,
    pub mappings: Vec<SourceMapping>,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassageSource {
    pub passage: Passage,
    pub sources: Vec<SourceExcerpt>,
}

pub trait TextExtractor: Send + Sync {
    fn identity(&self) -> ExtractorIdentity;
    fn extract(
        &self,
        bytes: &[u8],
        media_type: &str,
        limits: &TextLimits,
        cancellation: &AtomicBool,
    ) -> Result<ExtractedText>;
}
pub(super) fn check_cancel(cancellation: &AtomicBool) -> Result<()> {
    if cancellation.load(Ordering::Relaxed) {
        Err(AppError::new(
            ErrorKind::Cancelled,
            "text extraction cancelled",
            false,
        ))
    } else {
        Ok(())
    }
}
pub(super) fn invalid(message: &'static str) -> AppError {
    AppError::new(ErrorKind::InvalidInput, message, false)
}
pub(super) fn limit() -> AppError {
    AppError::new(
        ErrorKind::ResourceLimit,
        "text resource limit exceeded",
        false,
    )
}
