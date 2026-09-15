use super::*;
use lugus_financial::{
    domain::ProviderIdentity,
    resolution::catalog::CatalogSelection,
    selection::{FinancialRunEvidence, MarketRunEvidence},
    storage::{
        DocumentObservation, ScopedResolutionEvidence, SqliteRepository,
        bounded::{BoundedReadError, ReadLimits},
    },
};
pub trait EvidenceRepository: Send {
    fn repository_identity(&self) -> Result<String>;
    fn history_run(
        &self,
        p: &ProviderIdentity,
        id: i64,
        limits: ReadLimits,
    ) -> Result<lugus_financial::storage::history::HistoryRun>;
    fn history_page(
        &self,
        p: &ProviderIdentity,
        id: i64,
        offset: usize,
        limits: ReadLimits,
    ) -> Result<lugus_financial::storage::history::HistoryReadPage>;

    fn instrument_observation(
        &self,
        p: &ProviderIdentity,
        id: i64,
        limits: ReadLimits,
    ) -> Result<lugus_financial::instruments::InstrumentObservation>;
    fn financial_run(
        &self,
        p: &ProviderIdentity,
        id: i64,
        limits: ReadLimits,
    ) -> Result<FinancialRunEvidence>;
    fn market_run(
        &self,
        p: &ProviderIdentity,
        id: i64,
        limits: ReadLimits,
    ) -> Result<MarketRunEvidence>;
    fn resolution_run(
        &self,
        p: &ProviderIdentity,
        id: i64,
        limits: ReadLimits,
    ) -> Result<ScopedResolutionEvidence>;
    fn document(&self, observation: &DocumentObservation, max_bytes: usize) -> Result<Vec<u8>>;
    fn select_candidate(
        &mut self,
        p: &ProviderIdentity,
        run: i64,
        observation: i64,
        limits: ReadLimits,
    ) -> Result<CatalogSelection>;
}
pub(super) fn financial(e: lugus_financial::error::Error) -> AppError {
    use lugus_financial::error::ErrorKind as K;
    error(
        match e.kind {
            K::NotFound => ErrorKind::StaleReference,
            K::InvalidRequest => ErrorKind::InvalidInput,
            _ => ErrorKind::Storage,
        },
        "stored evidence is unavailable or incompatible",
    )
}
fn bounded(e: BoundedReadError) -> AppError {
    match e {
        BoundedReadError::LimitExceeded => limit(),
        BoundedReadError::Financial(e) => financial(e),
    }
}
impl EvidenceRepository for SqliteRepository {
    fn history_run(
        &self,
        p: &ProviderIdentity,
        id: i64,
        limits: ReadLimits,
    ) -> Result<lugus_financial::storage::history::HistoryRun> {
        lugus_financial::storage::history::HistoryRepository::history_run(self, p, id, limits)
            .map_err(bounded)
    }
    fn history_page(
        &self,
        p: &ProviderIdentity,
        id: i64,
        offset: usize,
        limits: ReadLimits,
    ) -> Result<lugus_financial::storage::history::HistoryReadPage> {
        lugus_financial::storage::history::HistoryRepository::history_page(
            self, p, id, offset, limits,
        )
        .map_err(bounded)
    }

    fn instrument_observation(
        &self,
        p: &ProviderIdentity,
        id: i64,
        limits: ReadLimits,
    ) -> Result<lugus_financial::instruments::InstrumentObservation> {
        self.bounded_instrument_observation(p, id, limits)
            .map_err(bounded)
    }
    fn repository_identity(&self) -> Result<String> {
        self.bounded_repository_identity(256).map_err(bounded)
    }
    fn financial_run(
        &self,
        p: &ProviderIdentity,
        id: i64,
        l: ReadLimits,
    ) -> Result<FinancialRunEvidence> {
        self.bounded_financial_run(p, id, l).map_err(bounded)
    }
    fn market_run(
        &self,
        p: &ProviderIdentity,
        id: i64,
        l: ReadLimits,
    ) -> Result<MarketRunEvidence> {
        self.bounded_market_run(p, id, l).map_err(bounded)
    }
    fn resolution_run(
        &self,
        p: &ProviderIdentity,
        id: i64,
        l: ReadLimits,
    ) -> Result<ScopedResolutionEvidence> {
        self.bounded_scoped_resolution(p, id, l).map_err(bounded)
    }
    fn document(&self, o: &DocumentObservation, max: usize) -> Result<Vec<u8>> {
        self.bounded_document(o, max).map_err(bounded)
    }
    fn select_candidate(
        &mut self,
        p: &ProviderIdentity,
        run: i64,
        observation: i64,
        l: ReadLimits,
    ) -> Result<CatalogSelection> {
        self.bounded_select_candidate(p, run, observation, l)
            .map_err(bounded)
    }
}
