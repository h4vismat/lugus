use super::*;
use lugus_financial::storage::history::HistoryReadPage;
impl Application {
    pub async fn history_evidence_page(
        &self,
        scope: &Scope,
        id: &str,
        page: PageRequest,
    ) -> Result<HistoryReadPage> {
        scope.validate()?;
        validate_id(id)?;
        let scope = scope.clone();
        let id = id.to_owned();
        self.store(move |s| s.history_evidence_page(&scope, &id, page))
            .await
    }
}
