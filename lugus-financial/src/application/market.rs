//! Explicit refresh always starts a new source snapshot at page one.
use crate::{
    capabilities::MarketDataProvider,
    domain::Validate,
    error::{Error, ErrorKind, Result},
    market_data::*,
    storage::market::MarketRepository,
};
use std::collections::HashSet;
pub async fn ingest_prices<R: MarketRepository, P: MarketDataProvider + ?Sized>(
    repo: &mut R,
    provider: &mut P,
    query: &PriceQuery,
) -> Result<i64> {
    query.validate()?;
    let run = repo.start_market_run(provider.identity(), query)?;
    let result = async {
        let mut query = query.clone();
        query.cursor = None;
        let mut seen = HashSet::new();
        let mut coverage = None;
        let mut last_date = None;
        loop {
            let page = provider.fetch_prices(&query).await?;
            page.validate_for(&query)?;
            if coverage
                .as_ref()
                .is_some_and(|previous| previous != &page.coverage)
                || page
                    .items
                    .first()
                    .is_some_and(|bar| last_date.is_some_and(|date| bar.date <= date))
            {
                return Err(Error::new(
                    ErrorKind::Protocol,
                    "market coverage changed or dates did not advance",
                ));
            }
            let cursor = super::next_cursor(page.next_cursor.clone(), &mut seen)?;
            repo.save_prices_page(run, &page)?;
            coverage = Some(page.coverage);
            last_date = page.items.last().map(|bar| bar.date);
            query.cursor = cursor;
            if query.cursor.is_none() {
                return Ok(());
            }
        }
    }
    .await;
    repo.finish_market_run(run, result.as_ref().err())?;
    result.map(|()| run)
}
