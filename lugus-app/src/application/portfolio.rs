use super::*;
use crate::portfolio::*;
mod prices;
pub(super) use prices::PortfolioJob;
impl Application {
    pub(crate) async fn authorized_portfolio_snapshot(
        &self,
        scope: Scope,
        id: String,
    ) -> Result<PortfolioSnapshot> {
        self.portfolio_effect(move |s| s.portfolio_authorized_snapshot(&scope, &id))
            .await
    }

    async fn portfolio_effect<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut dyn PortfolioStore) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = self.inner.store.clone();
        tokio::task::spawn_blocking(move || f(lock(&store)?.portfolio_store_mut()?))
            .await
            .map_err(|_| error(ErrorKind::Storage, "portfolio task failed"))?
    }
    pub async fn execute_portfolio(&self, r: PortfolioCommand) -> Result<PortfolioReceipt> {
        self.portfolio_effect(move |s| s.portfolio_execute(&r))
            .await
    }
    pub async fn preview_portfolio(&self, r: PortfolioCommand) -> Result<PortfolioPreview> {
        self.portfolio_effect(move |s| s.portfolio_preview(&r))
            .await
    }
    pub async fn portfolio_list(&self, p: PageRequest) -> Result<PortfolioPage<PortfolioHeader>> {
        self.portfolio_effect(move |s| s.portfolio_list(p)).await
    }
    pub async fn portfolio_overview(
        &self,
        id: String,
        account: Option<String>,
    ) -> Result<PortfolioView> {
        self.portfolio_effect(move |s| s.portfolio_overview(&id, account.as_deref()))
            .await
    }
    pub async fn portfolio_document(&self, id: String) -> Result<PortfolioDocument> {
        self.portfolio_effect(move |s| s.portfolio_document(&id))
            .await
    }
    pub async fn portfolio_audit(
        &self,
        id: String,
        p: PageRequest,
    ) -> Result<PortfolioPage<AuditEntry>> {
        self.portfolio_effect(move |s| s.portfolio_audit(&id, p))
            .await
    }
    pub async fn create_portfolio_snapshot(&self, r: SnapshotRequest) -> Result<PortfolioSnapshot> {
        self.portfolio_effect(move |s| s.portfolio_snapshot(&r))
            .await
    }
    pub async fn read_portfolio_snapshot(
        &self,
        c: String,
        id: String,
    ) -> Result<PortfolioSnapshot> {
        self.portfolio_effect(move |s| s.portfolio_read_snapshot(&c, &id))
            .await
    }
    pub async fn read_portfolio_snapshot_page(
        &self,
        c: String,
        id: String,
        section: String,
        p: PageRequest,
    ) -> Result<PortfolioPage<serde_json::Value>> {
        self.portfolio_effect(move |s| s.portfolio_snapshot_page(&c, &id, &section, p))
            .await
    }
}
impl Application {
    pub async fn portfolio_rows(
        &self,
        id: String,
        account: Option<String>,
        section: String,
        p: PageRequest,
        revision: Option<u64>,
    ) -> Result<PortfolioPage<serde_json::Value>> {
        if p.limit == 0 || p.limit > 200 || p.offset > 100_000 {
            return Err(crate::portfolio::invalid("invalid portfolio page"));
        }
        let doc = self.portfolio_document(id.clone()).await?;
        if revision.is_some_and(|r| r != doc.header.revision) {
            return Err(AppError::new(
                ErrorKind::Conflict,
                "portfolio changed while paging",
                false,
            ));
        }
        if account
            .as_ref()
            .is_some_and(|id| !doc.accounts.iter().any(|a| &a.id == id))
        {
            return Err(AppError::new(
                ErrorKind::ScopeMismatch,
                "account not in portfolio",
                false,
            ));
        }
        let mut rows = vec![];
        for a in doc
            .accounts
            .iter()
            .filter(|a| account.as_ref().is_none_or(|id| id == &a.id))
        {
            match section.as_str(){
    "accounts"=>rows.push(serde_json::json!({"id":a.id,"name":a.name,"revision":a.revision.to_string(),"start":a.ledger.start,"opening":a.ledger.opening})),
    "transactions"=>{let mut events=a.ledger.events.clone();events.sort_by_key(|e|(e.date,e.order));for e in events{rows.push(serde_json::json!({"account_id":a.id,"account_name":a.name,"event":e}));}},
    "lots"|"matches"=>{let state=lugus_portfolio::replay(&a.ledger,chrono::Utc::now().date_naive()).map_err(crate::portfolio::engine)?;
     if section=="lots"{for lot in state.lots{rows.push(serde_json::json!({"account_id":a.id,"account_name":a.name,"lot":lot}));}}else{for m in state.matches{rows.push(serde_json::json!({"account_id":a.id,"account_name":a.name,"sale_match":m}));}}
    },
    _=>return Err(crate::portfolio::invalid("unknown portfolio section"))
   }
        }
        if section == "transactions" {
            rows.sort_by(|a, b| {
                a["event"]["date"]
                    .as_str()
                    .cmp(&b["event"]["date"].as_str())
                    .then_with(|| {
                        a["event"]["order"]
                            .as_u64()
                            .cmp(&b["event"]["order"].as_u64())
                    })
            });
        }
        let end = p.offset + p.limit;
        let next_offset = (end < rows.len()).then_some(end);
        Ok(PortfolioPage {
            items: rows.into_iter().skip(p.offset).take(p.limit).collect(),
            next_offset,
            revision: doc.header.revision,
        })
    }
}
