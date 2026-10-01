use super::*;
use crate::{AppError, FetchReference, PageRequest, Result, Scope};
pub trait ComparisonStore: Send {
    fn lookup_request(&self, s: &Scope, r: &ComparisonRequest) -> Result<Option<ComparisonJob>>;
    fn acquire(&self, s: &Scope) -> Result<ComparisonLease>;
    fn begin(
        &mut self,
        s: &Scope,
        r: &ComparisonRequest,
        l: &ComparisonLease,
        p: &CapturedProviders,
    ) -> Result<ComparisonJob>;
    fn record_fetch(&mut self, s: &Scope, id: &str, f: &FetchReference) -> Result<()>;
    fn publish(&mut self, s: &Scope, id: &str, p: &PreparedComparison) -> Result<ComparisonJob>;
    fn finish(
        &mut self,
        s: &Scope,
        id: &str,
        state: ComparisonState,
        error: Option<AppError>,
    ) -> Result<ComparisonJob>;
    fn recover_interrupted(&mut self, s: &Scope, l: &ComparisonLease)
    -> Result<Vec<ComparisonJob>>;
    fn job(&self, s: &Scope, id: &str) -> Result<ComparisonJob>;
    fn read(&self, s: &Scope, id: &str) -> Result<ComparisonRecord>;
    fn list(&self, s: &Scope, p: PageRequest) -> Result<ComparisonPage<ComparisonSummary>>;
    fn package(&self, s: &Scope, id: &str) -> Result<ResearchPackage>;
    fn rows(&self, s: &Scope, id: &str, p: PageRequest) -> Result<ComparisonPage<ComparisonRow>>;
    fn sources(
        &self,
        s: &Scope,
        id: &str,
        p: PageRequest,
    ) -> Result<ComparisonPage<ComparisonSource>>;
    fn package_entries(
        &self,
        s: &Scope,
        id: &str,
        p: PageRequest,
    ) -> Result<ComparisonPage<ResearchPackageEntry>>;
}
