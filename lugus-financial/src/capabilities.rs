use crate::{domain::*, error::Result};
use async_trait::async_trait;

pub trait Provider {
    fn identity(&self) -> &ProviderIdentity;
}
#[async_trait]
pub trait FilingsProvider: Provider + Send {
    async fn list_filings(&mut self, query: &Query) -> Result<Page<Filing>>;
    async fn fetch_document(&mut self, source_url: &str, max_bytes: usize) -> Result<Document>;
}
#[async_trait]
pub trait FundamentalsProvider: Provider + Send {
    async fn fetch_facts(&mut self, query: &Query) -> Result<Page<Fact>>;
}

#[async_trait]
pub trait MarketDataProvider: Provider + Send {
    async fn fetch_prices(
        &mut self,
        query: &crate::market_data::PriceQuery,
    ) -> Result<crate::market_data::PricePage>;
}
