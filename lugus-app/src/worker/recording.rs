//! Captures receipts at the actual repository write boundary, including failed fetches.
use crate::{PriceQuery, ProviderIdentity, Query, WorkerRepository};
use lugus_financial::storage::bounded::{BoundedReadError, ReadLimits};
use lugus_financial::{
    domain::{Document, Fact, Filing, Metric, Page},
    error::{Error, Result},
    market_data::PricePage,
    resolution::{ResolutionPage, SearchQuery, SearchRequest, catalog::*},
    storage::{DocumentObservation, ObservationRetrieval, Repository, Snapshot, market::*},
};
use serde::{Deserialize, Serialize};
use std::cell::Cell;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    Financial,
    Market,
    Historical,
    Resolution,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReceipt {
    pub kind: RunKind,
    pub id: i64,
}

pub(super) struct Recording<'a> {
    pub inner: &'a mut dyn WorkerRepository,
    pub runs: Vec<RunReceipt>,
    pub document: Option<DocumentObservation>,
    pub instrument_observation: Option<lugus_financial::instruments::InstrumentObservation>,
    pub read_limits: ReadLimits,
    pub read_limited: Cell<bool>,
    pub finalization_failed: Cell<bool>,
    pub protocol_failed: bool,
    pub resolution_failure: Option<crate::AppError>,
}
impl Repository for Recording<'_> {
    fn start_run(
        &mut self,
        provider: &ProviderIdentity,
        query: &Query,
        operation: &str,
    ) -> Result<i64> {
        let id = self.inner.start_run(provider, query, operation)?;
        self.runs.push(RunReceipt {
            kind: RunKind::Financial,
            id,
        });
        Ok(id)
    }
    fn save_filings_page(&mut self, run: i64, page: &Page<Filing>) -> Result<()> {
        self.inner.save_filings_page(run, page)
    }
    fn save_facts_page(
        &mut self,
        run: i64,
        page: &Page<Fact>,
        mapper: &dyn Fn(&Fact) -> Option<Metric>,
    ) -> Result<()> {
        self.inner.save_facts_page(run, page, mapper)
    }
    fn finish_run(&mut self, run: i64, error: Option<&Error>) -> Result<()> {
        self.protocol_failed |=
            error.is_some_and(|error| error.kind == lugus_financial::error::ErrorKind::Protocol);
        let result = self.inner.finish_run(run, error);
        self.finalization_failed.set(result.is_err());
        result
    }
    fn snapshot(&self, provider: &ProviderIdentity, query: &Query) -> Result<Snapshot> {
        self.inner.snapshot(provider, query)
    }
    fn save_document(
        &mut self,
        provider: &ProviderIdentity,
        document: &Document,
        max_bytes: usize,
    ) -> Result<String> {
        let checksum = self.inner.save_document(provider, document, max_bytes)?;
        self.document = Some(DocumentObservation {
            provider: provider.clone(),
            checksum: checksum.clone(),
            source_url: document.source_url.clone(),
            media_type: document.media_type.clone(),
            retrieved_at: document.retrieved_at,
        });
        Ok(checksum)
    }
    fn stored_document(&self, checksum: &str) -> Result<Vec<u8>> {
        self.inner.stored_document(checksum)
    }
}
impl MarketRepository for Recording<'_> {
    fn start_market_run(&mut self, provider: &ProviderIdentity, query: &PriceQuery) -> Result<i64> {
        let id = self.inner.start_market_run(provider, query)?;
        self.runs.push(RunReceipt {
            kind: RunKind::Market,
            id,
        });
        Ok(id)
    }
    fn save_prices_page(&mut self, run: i64, page: &PricePage) -> Result<()> {
        self.inner.save_prices_page(run, page)
    }
    fn finish_market_run(&mut self, run: i64, error: Option<&Error>) -> Result<()> {
        self.protocol_failed |=
            error.is_some_and(|error| error.kind == lugus_financial::error::ErrorKind::Protocol);
        let result = self.inner.finish_market_run(run, error);
        self.finalization_failed.set(result.is_err());
        result
    }
    fn market_snapshot(
        &self,
        provider: &ProviderIdentity,
        query: &PriceQuery,
    ) -> Result<MarketSnapshot> {
        self.inner.market_snapshot(provider, query)
    }
    fn market_run_observations(&self, run: i64) -> Result<Vec<ObservationRetrieval>> {
        self.inner.market_run_observations(run)
    }
}
impl CatalogRepository for Recording<'_> {
    fn start_resolution_run(
        &mut self,
        provider: &ProviderIdentity,
        request: &SearchRequest,
    ) -> Result<i64> {
        let id = self.inner.start_resolution_run(provider, request)?;
        self.runs.push(RunReceipt {
            kind: RunKind::Resolution,
            id,
        });
        Ok(id)
    }
    fn save_resolution_page(
        &mut self,
        run: i64,
        request: &SearchRequest,
        page: &ResolutionPage,
    ) -> Result<()> {
        self.inner.save_resolution_page(run, request, page)
    }
    fn fail_resolution_run(&mut self, run: i64, message: &str) -> Result<()> {
        let result = self.inner.fail_resolution_run(run, message);
        self.finalization_failed.set(result.is_err());
        result
    }
    fn fail_resolution_run_with_error(&mut self, run: i64, error: &Error) -> Result<()> {
        self.protocol_failed |= error.kind == lugus_financial::error::ErrorKind::Protocol;
        self.resolution_failure = Some(crate::AppError::from(Error {
            kind: error.kind,
            message: String::new(),
            retry_after_seconds: error.retry_after_seconds,
        }));
        let result = self.inner.fail_resolution_run_with_error(run, error);
        self.finalization_failed.set(result.is_err());
        result
    }
    fn select_candidate(&mut self, run: i64, observation: i64) -> Result<CatalogSelection> {
        self.inner.select_candidate(run, observation)
    }
    fn catalog_selection(&self, selection: CatalogSelectionId) -> Result<CatalogSelection> {
        self.inner.catalog_selection(selection)
    }
    fn resolution_outcome(&self, run: i64) -> Result<ResolutionOutcome> {
        self.inner
            .bounded_resolution_outcome(run, self.read_limits)
            .map_err(|error| match error {
                BoundedReadError::LimitExceeded => {
                    self.read_limited.set(true);
                    Error::new(
                        lugus_financial::error::ErrorKind::InvalidRequest,
                        "resolution evidence exceeds read budget",
                    )
                }
                BoundedReadError::Financial(error) => error,
            })
    }
    fn search_catalog(&self, query: &SearchQuery) -> Result<Vec<CatalogEntry>> {
        self.inner.search_catalog(query)
    }
    fn catalog_history(&self, company: CatalogCompanyId) -> Result<Vec<CatalogRetrieval>> {
        self.inner.catalog_history(company)
    }
}

impl lugus_financial::instruments::InstrumentRepository for Recording<'_> {
    fn save_instrument_observation(
        &mut self,
        provider: &ProviderIdentity,
        request: &lugus_financial::instruments::InstrumentLookup,
        metadata: &lugus_financial::instruments::InstrumentMetadata,
    ) -> Result<lugus_financial::instruments::InstrumentObservation> {
        let observation = self
            .inner
            .save_instrument_observation(provider, request, metadata)?;
        self.instrument_observation = Some(observation.clone());
        Ok(observation)
    }
}

impl lugus_financial::storage::history::HistoryRepository for Recording<'_> {
    fn start_history_run(
        &mut self,
        p: &ProviderIdentity,
        q: &lugus_financial::historical_prices::HistoryQuery,
    ) -> Result<i64> {
        let id = self.inner.start_history_run(p, q)?;
        self.runs.push(RunReceipt {
            kind: RunKind::Historical,
            id,
        });
        Ok(id)
    }
    fn save_history_page(
        &mut self,
        run: i64,
        page: &lugus_financial::historical_prices::HistoryPage,
    ) -> Result<()> {
        self.inner.save_history_page(run, page)
    }
    fn finish_history_run(&mut self, run: i64, error: Option<&Error>) -> Result<()> {
        self.protocol_failed |=
            error.is_some_and(|e| e.kind == lugus_financial::error::ErrorKind::Protocol);
        let result = self.inner.finish_history_run(run, error);
        self.finalization_failed.set(result.is_err());
        result
    }
    fn history_run(
        &self,
        p: &ProviderIdentity,
        run: i64,
        limits: ReadLimits,
    ) -> lugus_financial::storage::bounded::ReadResult<lugus_financial::storage::history::HistoryRun>
    {
        self.inner.history_run(p, run, limits)
    }
    fn history_page(
        &self,
        p: &ProviderIdentity,
        run: i64,
        offset: usize,
        limits: ReadLimits,
    ) -> lugus_financial::storage::bounded::ReadResult<
        lugus_financial::storage::history::HistoryReadPage,
    > {
        self.inner.history_page(p, run, offset, limits)
    }
}
