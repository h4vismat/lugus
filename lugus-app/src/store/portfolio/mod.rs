use super::{SqliteApplicationStore, storage};
use crate::{AppError, ErrorKind, PageRequest, Result, portfolio::*};
use lugus_portfolio::*;
use rusqlite::{OptionalExtension, params};
mod prices;
mod snapshots;
mod write;
pub(crate) fn migrate(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    tx.execute_batch("CREATE TABLE portfolios(id TEXT PRIMARY KEY, revision INTEGER NOT NULL, payload TEXT NOT NULL);
 CREATE TABLE portfolio_event_ids(account_id TEXT NOT NULL, event_id TEXT NOT NULL, PRIMARY KEY(account_id,event_id));
 CREATE TABLE portfolio_history(portfolio_id TEXT NOT NULL REFERENCES portfolios(id),revision INTEGER NOT NULL,payload TEXT NOT NULL,audit TEXT NOT NULL,PRIMARY KEY(portfolio_id,revision));
 CREATE TABLE portfolio_requests(id TEXT PRIMARY KEY,input TEXT NOT NULL,receipt TEXT NOT NULL);
 CREATE TABLE portfolio_snapshots(id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL REFERENCES conversations(id),portfolio_id TEXT NOT NULL REFERENCES portfolios(id),payload TEXT NOT NULL);
 CREATE TABLE portfolio_snapshot_rows(snapshot_id TEXT NOT NULL REFERENCES portfolio_snapshots(id),section TEXT NOT NULL,ordinal INTEGER NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(snapshot_id,section,ordinal));
 CREATE TABLE portfolio_refreshes(id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE, input TEXT NOT NULL, payload TEXT NOT NULL);
 CREATE TABLE portfolio_prices(portfolio_id TEXT NOT NULL REFERENCES portfolios(id),instrument_id TEXT NOT NULL,receipt TEXT NOT NULL,PRIMARY KEY(portfolio_id,instrument_id));
 PRAGMA user_version=7;").map_err(storage)
}
fn conflict(message: &str) -> AppError {
    AppError::new(ErrorKind::Conflict, message, false)
}
fn scoped(message: &str) -> AppError {
    AppError::new(ErrorKind::ScopeMismatch, message, false)
}
fn bound<T: serde::Serialize>(value: &T, max: usize) -> Result<String> {
    super::json(value, max)
}
fn validate_page(p: PageRequest) -> Result<()> {
    if p.limit == 0 || p.limit > 200 || p.offset > 100_000 {
        Err(invalid("invalid portfolio page"))
    } else {
        Ok(())
    }
}
fn page<T>(mut items: Vec<T>, p: PageRequest, revision: u64) -> Result<PortfolioPage<T>> {
    validate_page(p)?;
    let next_offset = (items.len() > p.limit).then_some(p.offset + p.limit);
    items.truncate(p.limit);
    Ok(PortfolioPage {
        items,
        next_offset,
        revision,
    })
}

fn read(connection: &rusqlite::Connection, id: &str) -> Result<PortfolioDocument> {
    crate::portfolio::id(id)?;
    let payload: Option<String> = connection
        .query_row("SELECT payload FROM portfolios WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(storage)?;
    serde_json::from_str(&payload.ok_or_else(|| scoped("portfolio not found"))?).map_err(storage)
}
fn states(doc: &PortfolioDocument, account: Option<&str>, today: Day) -> Result<Vec<AccountState>> {
    if account.is_some_and(|id| !doc.accounts.iter().any(|a| a.id == id)) {
        return Err(scoped("account does not belong to portfolio"));
    }
    doc.accounts
        .iter()
        .filter(|a| account.is_none_or(|id| id == a.id))
        .map(|a| replay(&a.ledger, today).map_err(engine))
        .collect()
}
pub(crate) fn view(
    doc: &PortfolioDocument,
    account: Option<&str>,
    today: Day,
    prices: &[PriceInput],
) -> Result<PortfolioView> {
    let states = states(doc, account, today)?;
    let valuation = value_accounts(&states, prices, today).map_err(engine)?;
    let sum = |field: fn(&AccountState) -> &Decimal| {
        states
            .iter()
            .try_fold(Decimal::zero(), |v, s| v.checked_add(field(s)))
            .map_err(engine)
    };
    let accounts=doc.accounts.iter().filter(|a|account.is_none_or(|id|id==a.id)).map(|a|AccountSummary{id:a.id.clone(),name:a.name.clone(),start:a.ledger.start,revision:a.revision,simplified:matches!(&a.ledger.opening,Opening::Existing{lots,..} if lots.iter().any(|l|l.simplified))}).collect();
    Ok(PortfolioView {
        id: doc.header.id.clone(),
        name: doc.header.name.clone(),
        revision: doc.header.revision,
        as_of: today,
        accounts,
        instruments: doc.instruments.iter().filter(|i| account.is_none() || doc.accounts.iter().filter(|a|account==Some(a.id.as_str())).any(|a|a.ledger.events.iter().any(|e|e.kind.instrument_id()==Some(i.id.as_str())) || matches!(&a.ledger.opening,Opening::Existing{lots,..} if lots.iter().any(|l|l.instrument_id==i.id)))).cloned().collect(),
        valuation,
        realized: sum(|s| &s.realized)?,
        dividends: sum(|s| &s.dividends)?,
        standalone_fees: sum(|s| &s.standalone_fees)?,
        trade_fees: sum(|s| &s.trade_fees)?,
        deposits: sum(|s| &s.deposits)?,
        withdrawals: sum(|s| &s.withdrawals)?,
        price_status: vec![],
    })
}
impl PortfolioStore for SqliteApplicationStore {
    fn portfolio_refresh_begin(&mut self, r: &RefreshRequest) -> Result<(RefreshResult, bool)> {
        prices::begin(self, r)
    }
    fn portfolio_refresh_finish(&mut self, r: &RefreshResult) -> Result<RefreshResult> {
        prices::finish(self, r)
    }
    fn portfolio_refresh_read(&self, id: &str) -> Result<RefreshResult> {
        prices::read_refresh(self, id)
    }

    fn portfolio_authorized_snapshot(
        &self,
        scope: &crate::Scope,
        id: &str,
    ) -> Result<PortfolioSnapshot> {
        let run_id = scope
            .run_id
            .as_ref()
            .ok_or_else(|| scoped("portfolio tools require a conversation run"))?;
        let c: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM conversations WHERE workspace=?1",
                [&scope.workspace_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        let c = c.ok_or_else(|| scoped("conversation not found"))?;
        let run = crate::conversations::ConversationStore::run(self, &c, run_id)?;
        if !run.input.references.iter().any(|r|matches!(&r.reference,crate::conversations::SelectedReference::Portfolio{id:selected} if selected==id)){return Err(scoped("snapshot was not selected for this run"))}
        snapshots::read(self, &c, id)
    }

    fn portfolio_execute(&mut self, r: &PortfolioCommand) -> Result<PortfolioReceipt> {
        write::execute(self, r)
    }
    fn portfolio_preview(&self, r: &PortfolioCommand) -> Result<PortfolioPreview> {
        let (doc, _) = write::propose(self, r)?;
        let today = self.clock.now().date_naive();
        Ok(PortfolioPreview {
            view: view(&doc, None, today, &[])?,
            states: states(&doc, None, today)?,
        })
    }
    fn portfolio_document(&self, id: &str) -> Result<PortfolioDocument> {
        read(&self.connection, id)
    }
    fn portfolio_list(&self, p: PageRequest) -> Result<PortfolioPage<PortfolioHeader>> {
        validate_page(p)?;
        let mut stmt = self
            .connection
            .prepare("SELECT json_extract(payload,'$.header') FROM portfolios ORDER BY id LIMIT ?1 OFFSET ?2")
            .map_err(storage)?;
        let docs = stmt
            .query_map(params![(p.limit + 1) as i64, p.offset as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(storage)?
            .map(|v| serde_json::from_str::<PortfolioHeader>(&v.map_err(storage)?).map_err(storage))
            .collect::<Result<Vec<_>>>()?;
        page(docs, p, 0)
    }
    fn portfolio_overview(&self, id: &str, account: Option<&str>) -> Result<PortfolioView> {
        let doc = read(&self.connection, id)?;
        prices::current_view(self, &doc, account)
    }
    fn portfolio_audit(&self, id: &str, p: PageRequest) -> Result<PortfolioPage<AuditEntry>> {
        validate_page(p)?;
        let doc = read(&self.connection, id)?;
        let mut stmt = self
            .connection
            .prepare(
                "SELECT audit FROM portfolio_history WHERE portfolio_id=?1 ORDER BY revision DESC LIMIT ?2 OFFSET ?3",
            )
            .map_err(storage)?;
        let items = stmt
            .query_map(params![id, (p.limit + 1) as i64, p.offset as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(storage)?
            .map(|v| serde_json::from_str(&v.map_err(storage)?).map_err(storage))
            .collect::<Result<Vec<_>>>()?;
        page(items, p, doc.header.revision)
    }
    fn portfolio_snapshot(&mut self, r: &SnapshotRequest) -> Result<PortfolioSnapshot> {
        snapshots::create(self, r)
    }
    fn portfolio_read_snapshot(&self, c: &str, id: &str) -> Result<PortfolioSnapshot> {
        snapshots::read(self, c, id)
    }
    fn portfolio_snapshot_page(
        &self,
        c: &str,
        id: &str,
        section: &str,
        p: PageRequest,
    ) -> Result<PortfolioPage<serde_json::Value>> {
        snapshots::rows(self, c, id, section, p)
    }
}
