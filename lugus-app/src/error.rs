use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidInput,
    Unsupported,
    AmbiguousProvider,
    Unavailable,
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
            FinancialKind::InvalidRequest => {
                (ErrorKind::InvalidInput, false, "invalid financial request")
            }
            FinancialKind::Unsupported => (
                ErrorKind::Unsupported,
                false,
                "unsupported financial operation",
            ),
            FinancialKind::Configuration => (
                ErrorKind::Unavailable,
                false,
                "provider configuration is unavailable",
            ),
            FinancialKind::RateLimited => {
                (ErrorKind::RateLimited, true, "provider rate limit reached")
            }
            FinancialKind::Unavailable => (ErrorKind::Unavailable, true, "provider is unavailable"),
            FinancialKind::NotFound => (
                ErrorKind::MissingData,
                false,
                "financial data was not found",
            ),
            FinancialKind::MalformedData | FinancialKind::Protocol => (
                ErrorKind::Unavailable,
                false,
                "provider returned an invalid response",
            ),
            FinancialKind::Timeout => (ErrorKind::Timeout, true, "provider operation timed out"),
            FinancialKind::Persistence => (
                ErrorKind::Storage,
                false,
                "financial storage operation failed",
            ),
        };
        let mut result = Self::new(kind, message, retryable);
        result.retry_after_seconds = error.retry_after_seconds;
        result
    }
}
