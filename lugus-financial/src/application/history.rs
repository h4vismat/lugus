use crate::{
    capabilities::HistoricalPricesProvider, domain::Validate, error::Result,
    historical_prices::HistoryQuery, storage::history::HistoryRepository,
};
use std::collections::HashSet;
pub async fn ingest_history<R: HistoryRepository + ?Sized, P: HistoricalPricesProvider + ?Sized>(
    repo: &mut R,
    provider: &mut P,
    query: &HistoryQuery,
) -> Result<i64> {
    query.validate()?;
    let run = repo.start_history_run(provider.identity(), query)?;
    let result = async {
        let mut query = query.clone();
        query.cursor = None;
        let mut seen = HashSet::new();
        loop {
            let page = provider.fetch_history(&query).await?;
            page.validate_for(&query)?;
            let next = super::next_cursor(page.next_cursor.clone(), &mut seen)?;
            repo.save_history_page(run, &page).map_err(|error| {
                if error.kind == crate::error::ErrorKind::InvalidRequest {
                    crate::error::Error::new(
                        crate::error::ErrorKind::Protocol,
                        "historical page violates the immutable snapshot contract",
                    )
                } else {
                    error
                }
            })?;
            query.cursor = next;
            if query.cursor.is_none() {
                return Ok(());
            }
        }
    }
    .await;
    repo.finish_history_run(run, result.as_ref().err())?;
    result.map(|()| run)
}
