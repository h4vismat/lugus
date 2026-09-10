//! Immutable references and pure, typed presentation contracts.
use crate::*;
use chrono::{DateTime, Utc};
use lugus_financial::{
    domain::{Decimal, Filing},
    market_data::{PriceBar, PriceCoverage},
    resolution::catalog::CatalogEntry,
    selection::{Evidence, FactGroup, MetricQuery, PriceSeries, RunReference},
    storage::DocumentObservation,
};
use serde::{Deserialize, Serialize};

pub trait Clock: Send {
    fn now(&self) -> DateTime<Utc>;
}
pub trait IdSource: Send {
    fn next_id(&self) -> String;
}
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchReference {
    pub id: String,
    pub scope: Scope,
    pub provider: ProviderIdentity,
    pub repository_id: String,
    pub command: FetchCommand,
    pub runs: Vec<RunReceipt>,
    pub document: Option<DocumentObservation>,
    #[serde(default)]
    pub instrument_observation: Option<lugus_financial::instruments::InstrumentObservation>,
    #[serde(default)]
    pub binding_id: Option<String>,
    pub error: Option<AppError>,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DatasetProjection {
    Prices {
        run_id: i64,
        query: PriceQuery,
        series: PriceSeries,
    },
    Facts {
        run_id: i64,
        query: MetricQuery,
    },
    Filings {
        run_id: i64,
    },
    Resolution {
        run_id: i64,
    },
    Document,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetHeader {
    #[serde(default)]
    pub binding_id: Option<String>,
    pub id: String,
    pub workspace_id: String,
    pub repository_id: String,
    pub provider: ProviderIdentity,
    pub fetch_id: String,
    pub kind: DatasetKind,
    pub projection: DatasetProjection,
    pub query: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub row_count: usize,
    pub policy: Option<String>,
    pub selected_run: Option<RunReference>,
    pub coverage: Option<PriceCoverage>,
    pub limitations: Vec<String>,
    pub conflicts: Vec<String>,
    pub resolution_status: Option<String>,
    pub source_snapshot: Option<String>,
    pub source_coverage: Option<String>,
    pub document: Option<DocumentObservation>,
    pub error: Option<AppError>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DatasetRow {
    Price {
        evidence: Evidence<PriceBar>,
        value: Option<Decimal>,
    },
    Fact {
        group: FactGroup,
    },
    Filing {
        evidence: Evidence<Filing>,
    },
    Candidate {
        entry: CatalogEntry,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageRequest {
    pub offset: usize,
    pub limit: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetPage {
    pub header: DatasetHeader,
    pub rows: Vec<DatasetRow>,
    pub next_offset: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentRead {
    pub dataset_id: String,
    pub observation: DocumentObservation,
    pub offset: usize,
    pub total_bytes: usize,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenViewRequest {
    pub dataset_id: String,
    pub kind: ViewKind,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationStatus {
    Presented,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationResult {
    pub view_id: String,
    pub descriptor_revision: u32,
    pub status: PresentationStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewReceipt {
    pub id: String,
    pub workspace_id: String,
    pub request_id: String,
    pub dataset_id: String,
    pub kind: ViewKind,
    pub descriptor_revision: u32,
    pub accepted_at: DateTime<Utc>,
    pub presentation: Option<PresentationStatus>,
}
