use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidInput,
    Unsupported,
    AmbiguousProvider,
    Unavailable,
    AuthenticationRequired,
    NeedsAttention,
    Deactivated,
    MissingData,
    ScopeMismatch,
    StaleReference,
    Conflict,
    ResourceLimit,
    RateLimited,
    Cancelled,
    Storage,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct AppError {
    pub kind: ErrorKind,
    pub message: String,
    pub retryable: bool,
    pub retry_after_seconds: Option<u64>,
}

impl AppError {
    pub const MAX_MESSAGE_BYTES: usize = 1024;
    pub const MAX_SERIALIZED_JSON_BYTES: usize = Self::MAX_MESSAGE_BYTES * 2 + 256;

    pub fn new(kind: ErrorKind, message: impl Into<String>, retryable: bool) -> Self {
        let message = message.into();
        Self {
            kind,
            message: bounded_message(&message),
            retryable,
            retry_after_seconds: None,
        }
    }

    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        self.retry_after_seconds = Some(seconds);
        self
    }
}

fn bounded_message(message: &str) -> String {
    let mut bounded = String::with_capacity(message.len().min(AppError::MAX_MESSAGE_BYTES));
    for character in message.chars() {
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        if bounded.len() + character.len_utf8() > AppError::MAX_MESSAGE_BYTES {
            break;
        }
        bounded.push(character);
    }
    bounded
}

#[derive(Serialize)]
struct AppErrorRef<'a> {
    kind: ErrorKind,
    message: &'a str,
    retryable: bool,
    retry_after_seconds: Option<u64>,
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let message = bounded_message(&self.message);
        AppErrorRef {
            kind: self.kind,
            message: &message,
            retryable: self.retryable,
            retry_after_seconds: self.retry_after_seconds,
        }
        .serialize(serializer)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppErrorWire {
    kind: ErrorKind,
    message: String,
    retryable: bool,
    retry_after_seconds: Option<u64>,
}

impl<'de> Deserialize<'de> for AppError {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let wire = AppErrorWire::deserialize(deserializer)?;
        let mut error = Self::new(wire.kind, wire.message, wire.retryable);
        error.retry_after_seconds = wire.retry_after_seconds;
        Ok(error)
    }
}

impl From<lugus_financial::error::Error> for AppError {
    fn from(error: lugus_financial::error::Error) -> Self {
        use lugus_financial::error::ErrorKind as FinancialKind;
        let (kind, retryable, message) = match error.kind {
            FinancialKind::InvalidRequest => (
                ErrorKind::InvalidInput,
                false,
                "Invalid financial request. Check the provider's identifier namespace, date range, and filters before trying again.",
            ),
            FinancialKind::Unsupported => (
                ErrorKind::Unsupported,
                false,
                "This provider does not support the requested operation. Select an offered provider with that capability.",
            ),
            FinancialKind::Configuration => (
                ErrorKind::Unavailable,
                false,
                "Provider setup or access needs attention. Check its runtime, dependencies, and required source identity or credentials before retrying.",
            ),
            FinancialKind::RateLimited => (
                ErrorKind::RateLimited,
                true,
                "The data source is rate limiting requests. Wait for the indicated retry delay before starting a new fetch; saved evidence remains available.",
            ),
            FinancialKind::Unavailable => (
                ErrorKind::Unavailable,
                true,
                "The data source is temporarily unavailable. Try a new fetch later or use existing saved evidence with its retrieval date.",
            ),
            FinancialKind::NotFound => (
                ErrorKind::MissingData,
                false,
                "The source returned no matching data. Check the exact listing or company identifier and requested dates; this does not prove the company or instrument does not exist.",
            ),
            FinancialKind::MalformedData | FinancialKind::Protocol => (
                ErrorKind::Unavailable,
                false,
                "The provider returned an invalid response. Repeating the same request may not help; check the plugin or use saved evidence, preserving its source and retrieval date.",
            ),
            FinancialKind::Timeout => (
                ErrorKind::Timeout,
                true,
                "The data source timed out. Try a new fetch later; if it repeatedly times out, request a narrower date range and disclose the reduced coverage.",
            ),
            FinancialKind::Persistence => (
                ErrorKind::Storage,
                false,
                "Financial evidence could not be saved. Check local storage before fetching again; do not treat this request as saved evidence.",
            ),
        };
        let mut result = Self::new(kind, message, retryable);
        result.retry_after_seconds = error.retry_after_seconds;
        result
    }
}
