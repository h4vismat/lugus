use crate::error::{AppError, ErrorKind, Result};
use chrono::NaiveDate;
use lugus_financial::{
    domain::{CompanyId, Validate},
    market_data::InstrumentId,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::time::Duration;

pub use lugus_financial::{
    domain::{ProviderIdentity, Query},
    market_data::PriceQuery,
    resolution::LookupRequest,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub workspace_id: String,
    pub request_id: String,
    pub run_id: Option<String>,
}

impl Scope {
    pub const MAX_ID_BYTES: usize = 256;

    pub fn validate(&self) -> Result<()> {
        if valid_text(&self.workspace_id, Self::MAX_ID_BYTES)
            && valid_text(&self.request_id, Self::MAX_ID_BYTES)
            && self
                .run_id
                .as_deref()
                .is_none_or(|value| valid_text(value, Self::MAX_ID_BYTES))
        {
            Ok(())
        } else {
            Err(invalid("scope identifiers must be nonempty bounded text"))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub queue_capacity: usize,
    pub max_concurrent_jobs: usize,
    pub operation_timeout: Duration,
    pub max_pages_per_fetch: usize,
    pub max_items_per_fetch: usize,
    pub max_bytes_per_fetch: usize,
    pub max_document_bytes: usize,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_read_page_items: usize,
    pub max_read_page_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            queue_capacity: 64,
            max_concurrent_jobs: 1,
            operation_timeout: Duration::from_secs(60),
            max_pages_per_fetch: 100,
            max_items_per_fetch: 100_000,
            max_bytes_per_fetch: 128 * 1024 * 1024,
            max_document_bytes: 32 * 1024 * 1024,
            max_input_bytes: 1024 * 1024,
            max_output_bytes: 1024 * 1024,
            max_read_page_items: 1_000,
            max_read_page_bytes: 1024 * 1024,
        }
    }
}

impl Limits {
    pub const MAX_QUEUE_CAPACITY: usize = 100_000;
    pub const MAX_CONCURRENT_JOBS: usize = 1_024;
    pub const MAX_OPERATION_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
    pub const MAX_PAGES_PER_FETCH: usize = 10_000;
    pub const MAX_ITEMS_PER_FETCH: usize = 10_000_000;
    pub const MAX_BYTES: usize = 1024 * 1024 * 1024;
    pub const MIN_OUTPUT_BYTES: usize = AppError::MAX_SERIALIZED_JSON_BYTES;

    pub fn validate(&self) -> Result<()> {
        if (1..=Self::MAX_QUEUE_CAPACITY).contains(&self.queue_capacity)
            && (1..=Self::MAX_CONCURRENT_JOBS).contains(&self.max_concurrent_jobs)
            && (Duration::from_nanos(1)..=Self::MAX_OPERATION_TIMEOUT)
                .contains(&self.operation_timeout)
            && (1..=Self::MAX_PAGES_PER_FETCH).contains(&self.max_pages_per_fetch)
            && (1..=Self::MAX_ITEMS_PER_FETCH).contains(&self.max_items_per_fetch)
            && (1..=Self::MAX_BYTES).contains(&self.max_bytes_per_fetch)
            && (1..=Self::MAX_BYTES).contains(&self.max_document_bytes)
            && (1..=Self::MAX_BYTES).contains(&self.max_input_bytes)
            && (Self::MIN_OUTPUT_BYTES..=Self::MAX_BYTES).contains(&self.max_output_bytes)
            && (1..=Self::MAX_ITEMS_PER_FETCH).contains(&self.max_read_page_items)
            && (1..=Self::MAX_BYTES).contains(&self.max_read_page_bytes)
            && self.max_document_bytes <= self.max_bytes_per_fetch
            && self.max_read_page_items <= self.max_items_per_fetch
            && self.max_read_page_bytes <= self.max_output_bytes
        {
            Ok(())
        } else {
            Err(invalid("application limits are outside safe bounds"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Resolve,
    Lookup,
    Filings,
    Facts,
    Document,
    Prices,
}

impl Operation {
    pub fn capability(self) -> (&'static str, u32) {
        match self {
            Self::Resolve | Self::Lookup => ("company_resolution", 1),
            Self::Filings | Self::Document => ("filings", 1),
            Self::Facts => ("fundamentals", 1),
            Self::Prices => ("market_data", 1),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum FetchCommand {
    Resolve {
        instance_id: String,
        input: String,
    },
    Lookup {
        instance_id: String,
        request: LookupRequest,
    },
    Filings {
        instance_id: String,
        query: Query,
    },
    Facts {
        instance_id: String,
        query: Query,
    },
    Document {
        instance_id: String,
        source_url: String,
    },
    Prices {
        instance_id: String,
        query: PriceQuery,
    },
}

impl FetchCommand {
    pub const MAX_TEXT_BYTES: usize = 4096;
    pub const MAX_IDENTIFIER_COMPONENT_BYTES: usize = 128;

    pub fn instance_id(&self) -> &str {
        match self {
            Self::Resolve { instance_id, .. }
            | Self::Lookup { instance_id, .. }
            | Self::Filings { instance_id, .. }
            | Self::Facts { instance_id, .. }
            | Self::Document { instance_id, .. }
            | Self::Prices { instance_id, .. } => instance_id,
        }
    }

    pub fn operation(&self) -> Operation {
        match self {
            Self::Resolve { .. } => Operation::Resolve,
            Self::Lookup { .. } => Operation::Lookup,
            Self::Filings { .. } => Operation::Filings,
            Self::Facts { .. } => Operation::Facts,
            Self::Document { .. } => Operation::Document,
            Self::Prices { .. } => Operation::Prices,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if !valid_text(self.instance_id(), Scope::MAX_ID_BYTES) {
            return Err(AppError::new(
                ErrorKind::AmbiguousProvider,
                "an explicit provider instance is required",
                false,
            ));
        }
        match self {
            Self::Resolve { input, .. } => require_text(input, "invalid resolution input"),
            Self::Lookup { request, .. } => map_validation(request.validate()),
            Self::Filings { query, .. } | Self::Facts { query, .. } => {
                validate_identifier(&query.company.namespace, &query.company.value)?;
                map_validation(query.validate())?;
                if query.cursor.is_some() {
                    return Err(invalid("root fetch query cannot contain a cursor"));
                }
                if query.forms.len() > 100 || query.forms.iter().any(|form| !valid_text(form, 128))
                {
                    return Err(invalid("invalid filing form filter"));
                }
                Ok(())
            }
            Self::Document { source_url, .. } => {
                require_text(source_url, "invalid document source URL")
            }
            Self::Prices { query, .. } => {
                validate_identifier(&query.instrument.namespace, &query.instrument.value)?;
                map_validation(query.validate())?;
                if query.cursor.is_some() {
                    Err(invalid("root price query cannot contain a cursor"))
                } else {
                    Ok(())
                }
            }
        }
    }
}

fn validate_identifier(namespace: &str, value: &str) -> Result<()> {
    if valid_text(namespace, FetchCommand::MAX_IDENTIFIER_COMPONENT_BYTES)
        && valid_text(value, FetchCommand::MAX_IDENTIFIER_COMPONENT_BYTES)
    {
        Ok(())
    } else {
        Err(invalid("invalid provider-native identifier"))
    }
}

fn require_text(value: &str, message: &'static str) -> Result<()> {
    if valid_text(value, FetchCommand::MAX_TEXT_BYTES) {
        Ok(())
    } else {
        Err(invalid(message))
    }
}

fn valid_text(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

fn map_validation(result: lugus_financial::error::Result<()>) -> Result<()> {
    result.map_err(|_| invalid("invalid financial query"))
}

fn invalid(message: &'static str) -> AppError {
    AppError::new(ErrorKind::InvalidInput, message, false)
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum StrictFetchCommand {
    Resolve {
        instance_id: String,
        input: String,
    },
    Lookup {
        instance_id: String,
        request: StrictLookupRequest,
    },
    Filings {
        instance_id: String,
        query: StrictQuery,
    },
    Facts {
        instance_id: String,
        query: StrictQuery,
    },
    Document {
        instance_id: String,
        source_url: String,
    },
    Prices {
        instance_id: String,
        query: StrictPriceQuery,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictCompanyId {
    namespace: String,
    value: String,
}

impl From<StrictCompanyId> for CompanyId {
    fn from(value: StrictCompanyId) -> Self {
        Self {
            namespace: value.namespace,
            value: value.value,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictLookupRequest {
    identifier: StrictCompanyId,
}

impl From<StrictLookupRequest> for LookupRequest {
    fn from(value: StrictLookupRequest) -> Self {
        Self {
            identifier: value.identifier.into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictQuery {
    company: StrictCompanyId,
    filed_from: NaiveDate,
    filed_to: NaiveDate,
    #[serde(default)]
    forms: Vec<String>,
    #[serde(default)]
    cursor: Option<String>,
    page_size: usize,
}

impl From<StrictQuery> for Query {
    fn from(value: StrictQuery) -> Self {
        Self {
            company: value.company.into(),
            filed_from: value.filed_from,
            filed_to: value.filed_to,
            forms: value.forms,
            cursor: value.cursor,
            page_size: value.page_size,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictInstrumentId {
    namespace: String,
    value: String,
}

impl From<StrictInstrumentId> for InstrumentId {
    fn from(value: StrictInstrumentId) -> Self {
        Self {
            namespace: value.namespace,
            value: value.value,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictPriceQuery {
    instrument: StrictInstrumentId,
    start: NaiveDate,
    end: NaiveDate,
    #[serde(default)]
    cursor: Option<String>,
    page_size: usize,
}

impl From<StrictPriceQuery> for PriceQuery {
    fn from(value: StrictPriceQuery) -> Self {
        Self {
            instrument: value.instrument.into(),
            start: value.start,
            end: value.end,
            cursor: value.cursor,
            page_size: value.page_size,
        }
    }
}

impl<'de> Deserialize<'de> for FetchCommand {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Ok(match StrictFetchCommand::deserialize(deserializer)? {
            StrictFetchCommand::Resolve { instance_id, input } => {
                Self::Resolve { instance_id, input }
            }
            StrictFetchCommand::Lookup {
                instance_id,
                request,
            } => Self::Lookup {
                instance_id,
                request: request.into(),
            },
            StrictFetchCommand::Filings { instance_id, query } => Self::Filings {
                instance_id,
                query: query.into(),
            },
            StrictFetchCommand::Facts { instance_id, query } => Self::Facts {
                instance_id,
                query: query.into(),
            },
            StrictFetchCommand::Document {
                instance_id,
                source_url,
            } => Self::Document {
                instance_id,
                source_url,
            },
            StrictFetchCommand::Prices { instance_id, query } => Self::Prices {
                instance_id,
                query: query.into(),
            },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetKind {
    Prices,
    Facts,
    Filings,
    Resolution,
    Document,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    PriceChart,
    DataTable,
    Document,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}
