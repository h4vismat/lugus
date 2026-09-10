//! Source-reported instrument identity. Association with a company belongs to the host.
pub use crate::storage::instruments::InstrumentRepository;
use crate::{
    capabilities::Provider,
    domain::{CompanyId, ProviderIdentity, Validate},
    error::{Error, ErrorKind, Result},
    market_data::InstrumentId,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentLookup {
    #[serde(deserialize_with = "instrument_id")]
    pub instrument: InstrumentId,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKind {
    Equity,
    Other,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentMetadata {
    #[serde(deserialize_with = "instrument_id")]
    pub instrument: InstrumentId,
    pub issuer_name: Option<String>,
    pub ticker: Option<String>,
    #[serde(default, deserialize_with = "optional_company_id")]
    pub exchange: Option<CompanyId>,
    pub kind: Option<InstrumentKind>,
    #[serde(deserialize_with = "company_ids")]
    pub issuer_identifiers: Vec<CompanyId>,
    pub source_url: String,
    pub source_checksum: String,
    pub retrieved_at: DateTime<Utc>,
}
#[async_trait]
pub trait InstrumentProvider: Provider + Send {
    async fn lookup_instrument(&mut self, query: &InstrumentLookup) -> Result<InstrumentMetadata>;
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentObservation {
    pub id: i64,
    #[serde(deserialize_with = "provider_identity")]
    pub provider: ProviderIdentity,
    pub request: InstrumentLookup,
    pub metadata: InstrumentMetadata,
    pub recorded_at: DateTime<Utc>,
}
fn require(valid: bool, message: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::InvalidRequest, message))
    }
}
fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn identifier(namespace: &str, value: &str) -> Result<()> {
    require(
        text(namespace, 128) && text(value, 128),
        "invalid instrument lookup identifier",
    )
}
pub(crate) fn validate_provider(provider: &ProviderIdentity) -> Result<()> {
    require(
        [
            &provider.instance_id,
            &provider.plugin_id,
            &provider.plugin_version,
        ]
        .iter()
        .all(|value| text(value, 128)),
        "invalid instrument observation provider",
    )
}
impl Validate for InstrumentLookup {
    fn validate(&self) -> Result<()> {
        identifier(&self.instrument.namespace, &self.instrument.value)
    }
}
impl Validate for InstrumentMetadata {
    fn validate(&self) -> Result<()> {
        identifier(&self.instrument.namespace, &self.instrument.value)?;
        require(
            self.issuer_name
                .as_ref()
                .is_none_or(|value| text(value, 1024)),
            "invalid instrument issuer name",
        )?;
        require(
            self.ticker.as_ref().is_none_or(|value| text(value, 128)),
            "invalid instrument ticker",
        )?;
        if let Some(exchange) = &self.exchange {
            identifier(&exchange.namespace, &exchange.value)?;
        }
        require(
            self.issuer_identifiers.len() <= 16,
            "too many instrument issuer identifiers",
        )?;
        for id in &self.issuer_identifiers {
            identifier(&id.namespace, &id.value)?;
        }
        require(
            text(&self.source_url, 4096),
            "invalid instrument source URL",
        )?;
        require(
            self.source_checksum.len() == 64
                && self
                    .source_checksum
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "invalid instrument source checksum",
        )
    }
}
impl InstrumentMetadata {
    pub fn validate_for(&self, query: &InstrumentLookup) -> Result<()> {
        query.validate()?;
        self.validate()?;
        require(
            self.instrument == query.instrument,
            "instrument source identity does not match lookup",
        )
    }
}
// Local strict wire projections preserve existing CompanyId/InstrumentId semantics
// for older capabilities, while rejecting unknown fields at every lookup boundary.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identifier {
    namespace: String,
    value: String,
}
fn instrument_id<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<InstrumentId, D::Error> {
    let id = Identifier::deserialize(deserializer)?;
    Ok(InstrumentId {
        namespace: id.namespace,
        value: id.value,
    })
}
fn optional_company_id<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<CompanyId>, D::Error> {
    Ok(
        Option::<Identifier>::deserialize(deserializer)?.map(|id| CompanyId {
            namespace: id.namespace,
            value: id.value,
        }),
    )
}
fn company_ids<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<CompanyId>, D::Error> {
    Ok(Vec::<Identifier>::deserialize(deserializer)?
        .into_iter()
        .map(|id| CompanyId {
            namespace: id.namespace,
            value: id.value,
        })
        .collect())
}
pub(crate) fn provider_identity<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<ProviderIdentity, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Identity {
        instance_id: String,
        plugin_id: String,
        plugin_version: String,
    }
    let id = Identity::deserialize(deserializer)?;
    Ok(ProviderIdentity {
        instance_id: id.instance_id,
        plugin_id: id.plugin_id,
        plugin_version: id.plugin_version,
    })
}
