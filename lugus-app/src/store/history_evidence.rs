use super::*;
use lugus_financial::storage::{RunStatus, bounded::ReadLimits, history::HistoryReadPage};
impl SqliteApplicationStore {
    pub(super) fn read_history_evidence(
        &self,
        scope: &Scope,
        id: &str,
        page: PageRequest,
    ) -> Result<HistoryReadPage> {
        if page.limit == 0
            || page.limit > 200
            || page.limit > self.limits.max_read_page_items
            || page.offset > 100_000
        {
            return Err(limit());
        }
        let fetch = self.read_fetch(scope, id)?;
        let FetchCommand::HistoricalPrices { query, .. } = &fetch.command else {
            return Err(error(
                ErrorKind::InvalidInput,
                "fetch is not historical evidence",
            ));
        };
        if fetch.error.is_some()
            || fetch.repository_id != self.evidence.repository_identity()?
            || fetch.runs.len() != 1
            || fetch.runs[0].kind != RunKind::Historical
        {
            return Err(error(
                ErrorKind::StaleReference,
                "historical fetch is incomplete or incompatible",
            ));
        }
        let limits = ReadLimits {
            max_items: page.limit,
            max_bytes: self.limits.max_read_page_bytes,
        };
        let run = self
            .evidence
            .history_run(&fetch.provider, fetch.runs[0].id, limits)?;
        if run.status != RunStatus::Complete || &run.query != query {
            return Err(error(
                ErrorKind::StaleReference,
                "historical run does not match fetch",
            ));
        }
        self.evidence
            .history_page(&fetch.provider, run.id, page.offset, limits)
    }
}
