//! Deterministic, provider-independent USD portfolio accounting.
mod decimal;
mod domain;
mod fifo;
mod replay;
mod valuation;
pub use decimal::Decimal;
pub use domain::*;
pub use replay::replay;
pub use valuation::*;
pub type Day = chrono::NaiveDate;
pub type Result<T> = std::result::Result<T, PortfolioError>;
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortfolioError {
    #[error("invalid decimal number")]
    InvalidNumber,
    #[error("value exceeds supported precision")]
    Precision,
    #[error("arithmetic exceeds supported range")]
    Overflow,
    #[error("invalid ledger record: {0}")]
    InvalidRecord(String),
    #[error("duplicate or invalid event order")]
    InvalidOrder,
    #[error("insufficient cash at transaction {event_id}")]
    InsufficientCash { event_id: String },
    #[error("insufficient shares at transaction {event_id}")]
    InsufficientShares { event_id: String },
}
pub(crate) fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(PortfolioError::InvalidRecord(message.into()))
    }
}
