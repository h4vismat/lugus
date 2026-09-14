use super::*;
pub(super) fn begin(
    s: &mut SqliteApplicationStore,
    r: &RefreshRequest,
) -> Result<(RefreshResult, bool)> {
    crate::portfolio::id(&r.request_id)?;
    let input = bound(r, 1024 * 1024)?;
    let tx = rusqlite::Transaction::new_unchecked(
        &s.connection,
        rusqlite::TransactionBehavior::Immediate,
    )
    .map_err(storage)?;
    let prior: Option<(String, String)> = tx
        .query_row(
            "SELECT input,payload FROM portfolio_refreshes WHERE request_id=?1",
            [&r.request_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    if let Some((old, payload)) = prior {
        if old != input {
            return Err(conflict("refresh request ID reused"));
        }
        return Ok((serde_json::from_str(&payload).map_err(storage)?, false));
    }
    let d = super::read(&tx, &r.portfolio_id)?;
    if d.header.revision != r.expected_revision {
        return Err(conflict("portfolio changed before refresh"));
    }
    let result = RefreshResult {
        id: s.ids.next_id(),
        request: r.clone(),
        status: "running".into(),
        receipts: vec![],
    };
    tx.execute(
        "INSERT INTO portfolio_refreshes(id,request_id,input,payload) VALUES(?1,?2,?3,?4)",
        params![result.id, r.request_id, input, bound(&result, 1024 * 1024)?],
    )
    .map_err(storage)?;
    tx.commit().map_err(storage)?;
    Ok((result, true))
}
pub(super) fn read_refresh(s: &SqliteApplicationStore, id: &str) -> Result<RefreshResult> {
    let payload: Option<String> = s
        .connection
        .query_row(
            "SELECT payload FROM portfolio_refreshes WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    serde_json::from_str(&payload.ok_or_else(|| scoped("refresh not found"))?).map_err(storage)
}
pub(super) fn finish(s: &mut SqliteApplicationStore, r: &RefreshResult) -> Result<RefreshResult> {
    let tx = rusqlite::Transaction::new_unchecked(
        &s.connection,
        rusqlite::TransactionBehavior::Immediate,
    )
    .map_err(storage)?;
    let d = super::read(&tx, &r.request.portfolio_id)?;
    let mut result = r.clone();
    if d.header.revision != r.request.expected_revision {
        result.status = "stale_revision".into();
    } else {
        for incoming in &r.receipts {
            let mut price = incoming.clone();
            if price.price.is_none() {
                let old:Option<String>=tx.query_row("SELECT receipt FROM portfolio_prices WHERE portfolio_id=?1 AND instrument_id=?2",params![r.request.portfolio_id,price.instrument_id],|row|row.get(0)).optional().map_err(storage)?;
                if let Some(old) = old {
                    let mut saved: PortfolioPriceReceipt =
                        serde_json::from_str(&old).map_err(storage)?;
                    if saved.price.is_some() && saved.binding == price.binding {
                        saved.status = format!(
                            "Refresh issue: {}; last usable quote retained",
                            price.status
                        );
                        price = saved;
                    }
                }
            }
            // Preserve the last usable quote's original provenance on refresh failure.
            tx.execute("INSERT INTO portfolio_prices(portfolio_id,instrument_id,receipt) VALUES(?1,?2,?3) ON CONFLICT(portfolio_id,instrument_id) DO UPDATE SET receipt=excluded.receipt",params![r.request.portfolio_id,price.instrument_id,bound(&price,1024*1024)?]).map_err(storage)?;
        }
    }
    tx.execute(
        "UPDATE portfolio_refreshes SET payload=?1 WHERE id=?2",
        params![bound(&result, 2 * 1024 * 1024)?, r.id],
    )
    .map_err(storage)?;
    tx.commit().map_err(storage)?;
    Ok(result)
}
pub(super) fn current_view(
    s: &SqliteApplicationStore,
    d: &PortfolioDocument,
    account: Option<&str>,
) -> Result<PortfolioView> {
    let receipts = receipts(s, &d.header.id)?;
    let eligible = receipts
        .iter()
        .filter(|r| {
            d.instruments
                .iter()
                .any(|i| i.id == r.instrument_id && i.binding.as_ref() == Some(&r.binding))
        })
        .filter_map(|r| {
            let mut price=r.price.clone()?;
            let today=s.clock.now().date_naive();
            if price.date>today || d.accounts.iter().flat_map(|a|a.ledger.events.iter()).any(|e|e.date>price.date && matches!(&e.kind,EventKind::Split{instrument_id,..} if instrument_id==&r.instrument_id)){return None;}
            if matches!(price.basis,PriceBasis::Compatible{share_basis_date} if share_basis_date<=today){price.basis=PriceBasis::Compatible{share_basis_date:today};Some(price)}else{None}
        })
        .collect::<Vec<_>>();
    let mut result = view(d, account, s.clock.now().date_naive(), &eligible)?;
    result.price_status = receipts
        .iter()
        .filter(|r| result.instruments.iter().any(|i| i.id == r.instrument_id))
        .map(|r| format!("{}: {}", r.instrument_id, r.status))
        .collect();
    Ok(result)
}

pub(super) fn receipts(s: &SqliteApplicationStore, id: &str) -> Result<Vec<PortfolioPriceReceipt>> {
    let mut stmt = s
        .connection
        .prepare("SELECT receipt FROM portfolio_prices WHERE portfolio_id=?1")
        .map_err(storage)?;
    let receipts = stmt
        .query_map([id], |r| r.get::<_, String>(0))
        .map_err(storage)?
        .map(|v| {
            serde_json::from_str::<PortfolioPriceReceipt>(&v.map_err(storage)?).map_err(storage)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(receipts)
}
