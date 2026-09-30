use crate::{research::SubjectMention, *};
use chrono::{DateTime, NaiveDate, Utc};
use lugus_financial::{
    comparison::{AnnualRow, ComparisonIssue, InputRef, RevenueBasis},
    domain::{CompanyId, Fact},
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonRequest {
    pub request_id: String,
    pub subjects: [SubjectMention; 2],
    pub period_end: NaiveDate,
    pub years: u8,
    pub revenue_basis: RevenueBasis,
    #[serde(default)]
    pub question: Option<String>,
    #[serde(default)]
    pub facts_instance: Option<String>,
    #[serde(default)]
    pub resolution_instance: Option<String>,
    #[serde(default)]
    pub previous_id: Option<String>,
}
impl ComparisonRequest {
    pub fn validate(&self, today: NaiveDate) -> Result<()> {
        crate::conversations::validate_id(&self.request_id)?;
        lugus_financial::comparison::AnnualPolicy {
            period_end: self.period_end,
            years: self.years,
            revenue_basis: self.revenue_basis,
        }
        .validate()?;
        if self.period_end > today
            || today
                .checked_sub_months(chrono::Months::new(120))
                .is_none_or(|start| self.period_end < start)
        {
            return Err(invalid(
                "comparison endpoint must be within the past ten years",
            ));
        }
        if self.question.as_ref().is_some_and(|q| {
            q.len() > 4096
                || q.chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r'))
        }) {
            return Err(invalid("comparison question exceeds supported text bounds"));
        }
        for id in [
            &self.facts_instance,
            &self.resolution_instance,
            &self.previous_id,
        ]
        .into_iter()
        .flatten()
        {
            crate::conversations::validate_id(id)?;
        }
        crate::research::ResearchIntent {
            workflow: crate::research::Workflow::Compare,
            subjects: self.subjects.to_vec(),
            start: None,
            end: None,
            clarification: None,
        }
        .validate(today)
    }
    pub fn policy(&self) -> lugus_financial::comparison::AnnualPolicy {
        lugus_financial::comparison::AnnualPolicy {
            period_end: self.period_end,
            years: self.years,
            revenue_basis: self.revenue_basis,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedProviders {
    pub facts: ProviderIdentity,
    pub resolution: ProviderIdentity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedCompany {
    pub company: CompanyId,
    pub name: String,
    pub resolution_dataset_id: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonState {
    Running,
    Complete,
    Partial,
    Failed,
    Cancelled,
    Interrupted,
}
impl ComparisonState {
    pub fn terminal(self) -> bool {
        self != Self::Running
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonOwner {
    Local,
    External,
    None,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonJob {
    pub id: String,
    pub workspace_id: String,
    pub repository_id: String,
    pub request: ComparisonRequest,
    pub providers: CapturedProviders,
    pub state: ComparisonState,
    pub fetch_ids: Vec<String>,
    pub comparison_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<AppError>,
    pub owner: ComparisonOwner,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonRow {
    pub company: CompanyId,
    pub annual: AnnualRow,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonSource {
    pub company: CompanyId,
    pub metric: String,
    pub input: InputRef,
    pub fact: Fact,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ResearchPackageEntry {
    Dependency(DatasetHeader),
    Selection(lugus_financial::comparison::AnnualSelection),
    Calculation(ComparisonRow),
    Fetch(FetchReference),
    Limitation(ComparisonIssue),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchPackage {
    pub id: String,
    pub workspace_id: String,
    pub repository_id: String,
    pub schema_version: u32,
    pub policy: String,
    pub created_at: DateTime<Utc>,
    pub companies: [ResolvedCompany; 2],
    pub dependency_count: usize,
    pub row_count: usize,
    pub source_count: usize,
    pub entry_count: usize,
    pub fingerprint: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonRecord {
    pub id: String,
    pub package_id: String,
    pub request: ComparisonRequest,
    pub companies: [ResolvedCompany; 2],
    pub row_count: usize,
    pub source_count: usize,
    pub previous_id: Option<String>,
    pub state: ComparisonState,
    pub created_at: DateTime<Utc>,
    pub issues: Vec<ComparisonIssue>,
}
pub type ComparisonSummary = ComparisonRecord;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonPage<T> {
    pub items: Vec<T>,
    pub next_offset: Option<usize>,
}
/// Constructed only by the application; external store adapters receive read-only access.
#[derive(Debug, Clone)]
pub struct PreparedComparison {
    pub(crate) companies: [ResolvedCompany; 2],
    pub(crate) dependencies: Vec<String>,
    pub(crate) rows: Vec<ComparisonRow>,
    pub(crate) sources: Vec<ComparisonSource>,
    pub(crate) entries: Vec<ResearchPackageEntry>,
    pub(crate) issues: Vec<ComparisonIssue>,
    pub(crate) partial: bool,
}
impl PreparedComparison {
    pub fn companies(&self) -> &[ResolvedCompany; 2] {
        &self.companies
    }
    pub fn dependencies(&self) -> &[String] {
        &self.dependencies
    }
    pub fn rows(&self) -> &[ComparisonRow] {
        &self.rows
    }
    pub fn sources(&self) -> &[ComparisonSource] {
        &self.sources
    }
    pub fn entries(&self) -> &[ResearchPackageEntry] {
        &self.entries
    }
    pub fn issues(&self) -> &[ComparisonIssue] {
        &self.issues
    }
    pub fn partial(&self) -> bool {
        self.partial
    }
}
pub(crate) fn invalid(s: &str) -> AppError {
    AppError::new(ErrorKind::InvalidInput, s, false)
}
