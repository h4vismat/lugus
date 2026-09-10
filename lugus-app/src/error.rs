use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(deny_unknown_fields)]
#[error("{kind:?}: {message}")]
pub struct AppError {
    pub kind: ErrorKind,
    pub message: String,
    pub retryable: bool,
    pub retry_after_seconds: Option<u64>,
}

impl AppError {
    pub const MAX_MESSAGE_BYTES: usize = 1024;

    pub fn new(kind: ErrorKind, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            kind,
            message: bounded_message(message.into()),
            retryable,
            retry_after_seconds: None,
        }
    }

    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        self.retry_after_seconds = Some(seconds);
        self
    }
}

fn bounded_message(message: String) -> String {
    let sanitized = message
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    if sanitized.len() <= AppError::MAX_MESSAGE_BYTES {
        return sanitized;
    }
    let mut end = AppError::MAX_MESSAGE_BYTES;
    while !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    sanitized[..end].to_owned()
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
