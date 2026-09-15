use super::*;
use crate::ProviderIdentity;
use chrono::{DateTime, Utc};
use lugus_portfolio::{Day, HistoryIssue, PerformancePoint, PerformanceSummary};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRange {
    pub start: Option<Day>,
    pub end: Day,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryRefreshMode {
    Missing,
    Force,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortfolioHistoryRequest {
    pub request_id: String,
    pub portfolio_id: String,
    pub account_id: Option<String>,
    #[serde(with = "revision")]
    pub expected_revision: u64,
    pub range: HistoryRange,
    pub refresh: HistoryRefreshMode,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryKey {
    pub portfolio_id: String,
    #[serde(with = "revision")]
    pub revision: u64,
    pub account_id: Option<String>,
    pub requested_range: HistoryRange,
    pub calculation_version: u32,
    pub benchmark_provider: Option<ProviderIdentity>,
    pub bindings_fingerprint: String,
}
impl HistoryKey {
    pub fn new(
        doc: &PortfolioDocument,
        r: &PortfolioHistoryRequest,
        benchmark_provider: Option<ProviderIdentity>,
    ) -> Result<Self> {
        id(&r.request_id)?;
        id(&r.portfolio_id)?;
        if doc.header.id != r.portfolio_id || doc.header.revision != r.expected_revision {
            return Err(crate::AppError::new(
                crate::ErrorKind::Conflict,
                "portfolio changed before history request",
                false,
            ));
        }
        if r.account_id
            .as_ref()
            .is_some_and(|id| !doc.accounts.iter().any(|a| &a.id == id))
        {
            return Err(invalid("account does not belong to portfolio"));
        }
        if r.range
            .start
            .is_some_and(|s| s > r.range.end || (r.range.end - s).num_days() >= 100_000)
        {
            return Err(invalid("invalid history range"));
        }
        Ok(Self {
            portfolio_id: r.portfolio_id.clone(),
            revision: r.expected_revision,
            account_id: r.account_id.clone(),
            requested_range: r.range.clone(),
            calculation_version: 1,
            benchmark_provider,
            bindings_fingerprint: history_bindings(doc)?,
        })
    }
}
pub fn history_bindings(doc: &PortfolioDocument) -> Result<String> {
    lugus_financial::domain::fingerprint(&(&doc.instruments, &doc.benchmark_instance_id))
        .map_err(crate::AppError::from)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatus {
    Running,
    Complete,
    Partial,
    Failed,
    Cancelled,
    TimedOut,
    StaleRevision,
    Interrupted,
    InterruptedOrExternal,
}
impl HistoryStatus {
    pub fn published(&self) -> bool {
        matches!(self, Self::Complete | Self::Partial)
    }
    pub(crate) fn text(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::StaleRevision => "stale_revision",
            Self::Interrupted => "interrupted",
            Self::InterruptedOrExternal => "interrupted_or_external",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEvidenceRef {
    #[serde(default)]
    pub manifest: Option<lugus_financial::historical_prices::HistoryManifest>,
    #[serde(default)]
    pub source_url: Option<String>,
    pub instrument_id: Option<String>,
    pub fetch_id: String,
    pub run_id: String,
    pub provider: ProviderIdentity,
    pub manifest_fingerprint: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioHistoryResult {
    pub id: String,
    pub key: HistoryKey,
    pub status: HistoryStatus,
    pub baseline: Option<Day>,
    pub effective_end: Option<Day>,
    pub summary: PerformanceSummary,
    pub row_count: usize,
    pub evidence: Vec<HistoryEvidenceRef>,
    pub issues: Vec<HistoryIssue>,
    pub issue_count: usize,
    pub input_fingerprint: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioHistoryPage {
    pub result_id: String,
    pub key: HistoryKey,
    pub items: Vec<PerformancePoint>,
    pub next_offset: Option<usize>,
}
