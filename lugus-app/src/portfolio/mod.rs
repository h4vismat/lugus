//! Durable user portfolio commands, read models and accounting boundaries.
mod prices;
mod types;
use crate::{PageRequest, Result};
pub use prices::yfinance_price_input;
pub use types::*;
pub trait PortfolioStore: Send {
    fn portfolio_refresh_begin(&mut self, r: &RefreshRequest) -> Result<(RefreshResult, bool)>;
    fn portfolio_refresh_finish(&mut self, r: &RefreshResult) -> Result<RefreshResult>;
    fn portfolio_refresh_read(&self, id: &str) -> Result<RefreshResult>;

    fn portfolio_authorized_snapshot(
        &self,
        scope: &crate::Scope,
        id: &str,
    ) -> Result<PortfolioSnapshot>;

    fn portfolio_execute(&mut self, request: &PortfolioCommand) -> Result<PortfolioReceipt>;
    fn portfolio_preview(&self, request: &PortfolioCommand) -> Result<PortfolioPreview>;
    fn portfolio_list(&self, page: PageRequest) -> Result<PortfolioPage<PortfolioHeader>>;
    fn portfolio_overview(&self, id: &str, account: Option<&str>) -> Result<PortfolioView>;
    fn portfolio_document(&self, id: &str) -> Result<PortfolioDocument>;
    fn portfolio_audit(&self, id: &str, page: PageRequest) -> Result<PortfolioPage<AuditEntry>>;
    fn portfolio_snapshot(&mut self, request: &SnapshotRequest) -> Result<PortfolioSnapshot>;
    fn portfolio_read_snapshot(&self, conversation: &str, id: &str) -> Result<PortfolioSnapshot>;
    fn portfolio_snapshot_page(
        &self,
        conversation: &str,
        id: &str,
        section: &str,
        page: PageRequest,
    ) -> Result<PortfolioPage<serde_json::Value>>;
}
pub(crate) fn invalid(s: impl Into<String>) -> crate::AppError {
    crate::AppError::new(crate::ErrorKind::InvalidInput, s, false)
}
pub(crate) fn engine(e: lugus_portfolio::PortfolioError) -> crate::AppError {
    invalid(e.to_string())
}
pub(crate) fn id(s: &str) -> Result<()> {
    if s.is_empty() || s.len() > 256 || s.chars().any(char::is_control) {
        Err(invalid("invalid portfolio identifier"))
    } else {
        Ok(())
    }
}
pub(crate) fn name(s: &str) -> Result<()> {
    id(s)?;
    if s.trim().is_empty() {
        Err(invalid("name must not be blank"))
    } else {
        Ok(())
    }
}
