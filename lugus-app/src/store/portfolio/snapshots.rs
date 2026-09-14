use super::*;
use std::collections::BTreeMap;
pub(super) fn create(
    s: &mut SqliteApplicationStore,
    r: &SnapshotRequest,
) -> Result<PortfolioSnapshot> {
    let input = bound(r, 1024 * 1024)?;
    crate::portfolio::id(&r.request_id)?;
    let tx = rusqlite::Transaction::new_unchecked(
        &s.connection,
        rusqlite::TransactionBehavior::Immediate,
    )
    .map_err(storage)?;
    let prior: Option<(String, String)> = tx
        .query_row(
            "SELECT input,receipt FROM portfolio_requests WHERE id=?1",
            [&r.request_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    if let Some((old, receipt)) = prior {
        if old != input {
            return Err(conflict("snapshot request ID reused"));
        }
        return serde_json::from_str(&receipt).map_err(storage);
    }
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
            [&r.conversation_id],
            |row| row.get(0),
        )
        .map_err(storage)?;
    if !exists {
        return Err(scoped("conversation not found"));
    }
    let d = super::read(&tx, &r.portfolio_id)?;
    if d.header.revision != r.expected_revision {
        return Err(conflict("portfolio changed before selection"));
    }
    let today = s.clock.now().date_naive();
    let summary = super::prices::current_view(s, &d, r.account_id.as_deref())?;
    let states = states(&d, r.account_id.as_deref(), today)?;
    let mut sections: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    sections.insert(
        "holdings".into(),
        summary
            .valuation
            .holdings
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<_, _>>()
            .map_err(storage)?,
    );
    for state in &states {
        for lot in &state.lots {
            sections
                .entry("lots".into())
                .or_default()
                .push(serde_json::json!({"account_id":state.account_id,"lot":lot}));
        }
        for m in &state.matches {
            sections
                .entry("matches".into())
                .or_default()
                .push(serde_json::json!({"account_id":state.account_id,"sale_match":m}));
        }
    }
    for a in d
        .accounts
        .iter()
        .filter(|a| r.account_id.as_ref().is_none_or(|id| id == &a.id))
    {
        for event in &a.ledger.events {
            sections
                .entry("transactions".into())
                .or_default()
                .push(serde_json::json!({"account_id":a.id,"event":event}));
        }
    }
    bound(&sections, 16 * 1024 * 1024)?;
    let prices = super::prices::receipts(s, &d.header.id)?
        .into_iter()
        .filter(|r| summary.instruments.iter().any(|i| i.id == r.instrument_id))
        .collect();
    let snapshot = PortfolioSnapshot {
        prices,
        id: s.ids.next_id(),
        conversation_id: r.conversation_id.clone(),
        created_at: s.clock.now(),
        calculation_version: 1,
        summary,
        row_counts: sections.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
    };
    let payload = bound(
        &snapshot,
        s.conversation_limits
            .selected_bytes
            .min(s.limits.max_output_bytes),
    )?;
    tx.execute("INSERT INTO portfolio_snapshots(id,conversation_id,portfolio_id,payload) VALUES(?1,?2,?3,?4)",params![snapshot.id,r.conversation_id,r.portfolio_id,payload]).map_err(storage)?;
    for (section, rows) in sections {
        for (ordinal, row) in rows.into_iter().enumerate() {
            tx.execute("INSERT INTO portfolio_snapshot_rows(snapshot_id,section,ordinal,payload) VALUES(?1,?2,?3,?4)",params![snapshot.id,section,ordinal as i64,bound(&row,s.limits.max_output_bytes)?]).map_err(storage)?;
        }
    }
    tx.execute(
        "INSERT INTO portfolio_requests(id,input,receipt) VALUES(?1,?2,?3)",
        params![r.request_id, input, payload],
    )
    .map_err(storage)?;
    tx.commit().map_err(storage)?;
    Ok(snapshot)
}
pub(super) fn read(s: &SqliteApplicationStore, c: &str, id: &str) -> Result<PortfolioSnapshot> {
    let payload: Option<String> = s
        .connection
        .query_row(
            "SELECT payload FROM portfolio_snapshots WHERE id=?1 AND conversation_id=?2",
            params![id, c],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    serde_json::from_str(
        &payload.ok_or_else(|| scoped("snapshot not selected for this conversation"))?,
    )
    .map_err(storage)
}
pub(super) fn rows(
    s: &SqliteApplicationStore,
    c: &str,
    id: &str,
    section: &str,
    p: PageRequest,
) -> Result<PortfolioPage<serde_json::Value>> {
    let snapshot = read(s, c, id)?;
    if !["holdings", "lots", "matches", "transactions"].contains(&section) {
        return Err(invalid("unknown snapshot section"));
    }
    if p.limit == 0 || p.limit > 200 || p.offset > 100_000 {
        return Err(invalid("invalid page"));
    }
    let mut stmt=s.connection.prepare("SELECT payload FROM portfolio_snapshot_rows WHERE snapshot_id=?1 AND section=?2 ORDER BY ordinal LIMIT ?3 OFFSET ?4").map_err(storage)?;
    let items = stmt
        .query_map(params![id, section, p.limit as i64, p.offset as i64], |r| {
            r.get::<_, String>(0)
        })
        .map_err(storage)?
        .map(|v| serde_json::from_str(&v.map_err(storage)?).map_err(storage))
        .collect::<Result<Vec<_>>>()?;
    let end = p.offset + items.len();
    let result = PortfolioPage {
        items,
        next_offset: (end < *snapshot.row_counts.get(section).unwrap_or(&0)).then_some(end),
        revision: snapshot.summary.revision,
    };
    bound(&result, s.limits.max_output_bytes)?;
    Ok(result)
}
