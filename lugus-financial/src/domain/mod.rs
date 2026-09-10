//! Immutable, source-independent values. Network and storage effects live elsewhere.
use crate::error::{Error, ErrorKind, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

pub trait Validate {
    fn validate(&self) -> Result<()>;
}
fn require(valid: bool, message: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::InvalidRequest, message))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CompanyId {
    pub namespace: String,
    pub value: String,
}
impl Validate for CompanyId {
    fn validate(&self) -> Result<()> {
        require(
            !self.namespace.trim().is_empty() && !self.value.trim().is_empty(),
            "company namespace and value are required",
        )
    }
}

/// Exact, plain decimal. Construction and deserialization enforce the same grammar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Decimal(String);
impl Decimal {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let unsigned = value.strip_prefix('-').unwrap_or(&value);
        let mut parts = unsigned.split('.');
        let integer = parts.next().unwrap_or("");
        let fraction = parts.next();
        require(
            !integer.is_empty()
                && integer.bytes().all(|b| b.is_ascii_digit())
                && fraction.is_none_or(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                && parts.next().is_none(),
            "expected a plain exact decimal string",
        )?;
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Period {
    Instant { date: NaiveDate },
    Duration { start: NaiveDate, end: NaiveDate },
}
impl Validate for Period {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Duration { start, end } => require(start <= end, "period start exceeds end"),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    pub company: CompanyId,
    pub filed_from: NaiveDate,
    pub filed_to: NaiveDate,
    #[serde(default)]
    pub forms: Vec<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    pub page_size: usize,
}
impl Validate for Query {
    fn validate(&self) -> Result<()> {
        self.company.validate()?;
        require(
            self.filed_from <= self.filed_to,
            "filing date range is reversed",
        )?;
        require(
            (1..=1000).contains(&self.page_size),
            "page_size must be between 1 and 1000",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub company: CompanyId,
    pub namespace: String,
    pub concept: String,
    pub label: Option<String>,
    pub value: Decimal,
    pub unit: String,
    pub period: Period,
    pub filing_id: String,
    pub form: String,
    pub filed: NaiveDate,
    pub fiscal_year: Option<i32>,
    pub fiscal_period: Option<String>,
    pub source_url: String,
    pub retrieved_at: DateTime<Utc>,
}
impl Validate for Fact {
    fn validate(&self) -> Result<()> {
        self.company.validate()?;
        self.period.validate()?;
        require(
            [
                &self.namespace,
                &self.concept,
                &self.unit,
                &self.filing_id,
                &self.form,
                &self.source_url,
            ]
            .iter()
            .all(|s| !s.trim().is_empty()),
            "fact source fields must not be empty",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Filing {
    pub company: CompanyId,
    pub filing_id: String,
    pub form: String,
    pub filed: NaiveDate,
    pub report_date: Option<NaiveDate>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub primary_document: Option<String>,
    pub source_url: String,
    pub retrieved_at: DateTime<Utc>,
}
impl Validate for Filing {
    fn validate(&self) -> Result<()> {
        self.company.validate()?;
        require(
            [&self.filing_id, &self.form, &self.source_url]
                .iter()
                .all(|s| !s.trim().is_empty()),
            "filing source fields must not be empty",
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub source_url: String,
    pub media_type: String,
    pub content_base64: String,
    pub retrieved_at: DateTime<Utc>,
}
impl Document {
    pub fn bytes(&self, max_bytes: usize) -> Result<Vec<u8>> {
        use base64::Engine;
        require(
            !self.source_url.is_empty() && !self.media_type.is_empty(),
            "document metadata is required",
        )?;
        require(
            self.content_base64.len()
                <= max_bytes
                    .saturating_add(2)
                    .saturating_div(3)
                    .saturating_mul(4),
            "document exceeds size limit",
        )?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&self.content_base64)
            .map_err(|_| Error::new(ErrorKind::MalformedData, "invalid document base64"))?;
        require(bytes.len() <= max_bytes, "document exceeds size limit")?;
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderIdentity {
    pub instance_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metric {
    pub id: String,
    pub mapping_version: String,
}

/// Conservative mappings. Unknown namespaces and wrong period semantics remain unmapped.
pub fn map_metric(fact: &Fact) -> Option<Metric> {
    if fact.namespace != "us-gaap" {
        return None;
    }
    let instant = matches!(fact.period, Period::Instant { .. });
    let id = match (fact.concept.as_str(), instant) {
        ("Assets", true) => "assets",
        ("Liabilities", true) => "liabilities",
        ("StockholdersEquity", true) => "equity",
        ("NetIncomeLoss", false) => "net_income",
        ("NetCashProvidedByUsedInOperatingActivities", false) => "operating_cash_flow",
        _ => return None,
    };
    Some(Metric {
        id: id.into(),
        mapping_version: "us-gaap:1".into(),
    })
}

pub fn fingerprint<T: Serialize>(value: &T) -> Result<String> {
    // serde_json's default map is a BTreeMap: recursively stable key ordering.
    let canonical = serde_json::to_value(value)?;
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&canonical)?)
    ))
}
fn observation_fingerprint<T: Serialize>(value: &T) -> Result<String> {
    let mut value = serde_json::to_value(value)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("retrieved_at");
    }
    fingerprint(&value)
}
impl Fact {
    pub fn fingerprint(&self) -> Result<String> {
        observation_fingerprint(self)
    }
}
impl Filing {
    pub fn fingerprint(&self) -> Result<String> {
        observation_fingerprint(self)
    }
}
