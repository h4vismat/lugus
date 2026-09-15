use super::*;
use crate::historical_prices::{HistoryPage, HistoryQuery};

#[async_trait]
impl HistoricalPricesProvider for Plugin {
    async fn fetch_history(&mut self, query: &HistoryQuery) -> Result<HistoryPage> {
        self.supports("historical_prices")?;
        query.validate()?;
        let page: HistoryPage = self
            .call("historical_prices.daily", serde_json::to_value(query)?)
            .await?;
        if page.validate_for(query).is_err() {
            let _ = self.close().await;
            return Err(Error::new(
                ErrorKind::Protocol,
                "invalid historical-price page",
            ));
        }
        Ok(page)
    }
}
