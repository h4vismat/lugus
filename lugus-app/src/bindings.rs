//! Application-owned, source-supported instrument associations.
pub mod policy;
use crate::Scope;
use chrono::{DateTime, Utc};
use lugus_financial::{
    instruments::InstrumentObservation,
    resolution::{Listing, catalog::CatalogEntry},
};
pub use policy::{BindingAssessment, assess_binding};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindRequest {
    pub company_dataset_id: String,
    pub company_observation_id: i64,
    #[serde(deserialize_with = "strict_listing")]
    pub listing: Listing,
    pub instrument_fetch_id: String,
    pub supersedes: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeBindingRequest {
    pub binding_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingRecord {
    pub id: String,
    pub scope: Scope,
    pub repository_id: String,
    pub company_dataset_id: String,
    pub company: CatalogEntry,
    pub listing: Listing,
    pub instrument_fetch_id: String,
    pub instrument: InstrumentObservation,
    pub policy: String,
    pub reasons: Vec<String>,
    pub supersedes: Option<String>,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BindingStatus {
    Active,
    Superseded { binding_id: String },
    Revoked,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingView {
    pub record: BindingRecord,
    pub status: BindingStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingPage {
    pub bindings: Vec<BindingView>,
    pub next_offset: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingEvent {
    pub scope: Scope,
    pub binding_id: String,
    pub status: BindingStatus,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingHistoryPage {
    pub events: Vec<BindingEvent>,
    pub next_offset: Option<usize>,
}

fn strict_listing<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Listing, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Identifier {
        namespace: String,
        value: String,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Selected {
        ticker: Identifier,
        exchange: Option<Identifier>,
    }
    let selected = Selected::deserialize(deserializer)?;
    let convert = |id: Identifier| lugus_financial::domain::CompanyId {
        namespace: id.namespace,
        value: id.value,
    };
    Ok(Listing {
        ticker: convert(selected.ticker),
        exchange: selected.exchange.map(convert),
    })
}
