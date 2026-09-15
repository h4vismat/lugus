use super::*;
fn transaction(s: &SqliteApplicationStore) -> Result<rusqlite::Transaction<'_>> {
    rusqlite::Transaction::new_unchecked(&s.connection, rusqlite::TransactionBehavior::Immediate)
        .map_err(storage)
}
fn result(c: &rusqlite::Connection, p: &str, id: &str) -> Result<PortfolioHistoryResult> {
    crate::portfolio::id(p)?;
    crate::portfolio::id(id)?;
    let payload: Option<String> = c
        .query_row(
            "SELECT payload FROM portfolio_history_jobs WHERE id=?1 AND portfolio_id=?2",
            params![id, p],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    serde_json::from_str(
        &payload.ok_or_else(|| scoped("history result does not belong to portfolio"))?,
    )
    .map_err(storage)
}
fn update(c: &rusqlite::Connection, r: &PortfolioHistoryResult) -> Result<()> {
    c.execute(
        "UPDATE portfolio_history_jobs SET payload=?1,status=?2 WHERE id=?3",
        params![bound(r, 512 * 1024)?, r.status.text(), r.id],
    )
    .map_err(storage)?;
    Ok(())
}
pub(super) fn begin(
    s: &mut SqliteApplicationStore,
    r: &PortfolioHistoryRequest,
    key: &HistoryKey,
    lease: &HistoryLease,
) -> Result<(PortfolioHistoryResult, bool)> {
    if !lease.matches(&s.store_key, &r.portfolio_id) {
        return Err(scoped("history execution lease mismatch"));
    }
    let input = bound(r, 16384)?;
    let tx = transaction(s)?;
    let old: Option<(String, String)> = tx
        .query_row(
            "SELECT input,payload FROM portfolio_history_jobs WHERE request_id=?1",
            [&r.request_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    if let Some((prior, payload)) = old {
        if prior != input {
            return Err(conflict("history request ID reused"));
        }
        return Ok((serde_json::from_str(&payload).map_err(storage)?, false));
    }
    let doc = read(&tx, &r.portfolio_id)?;
    if &HistoryKey::new(&doc, r, key.benchmark_provider.clone())? != key {
        return Err(conflict("history input changed"));
    }
    // Only the process holding the OS lease can recover an abandoned durable job.
    let old: Option<String> = tx
        .query_row(
            "SELECT id FROM portfolio_history_jobs WHERE portfolio_id=?1 AND status='running'",
            [&r.portfolio_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    if let Some(id) = old {
        let mut old = result(&tx, &r.portfolio_id, &id)?;
        old.status = HistoryStatus::Interrupted;
        old.finished_at = Some(s.clock.now());
        update(&tx, &old)?;
    }
    let result = PortfolioHistoryResult {
        id: s.ids.next_id(),
        key: key.clone(),
        status: HistoryStatus::Running,
        baseline: None,
        effective_end: None,
        summary: PerformanceSummary::default(),
        row_count: 0,
        evidence: vec![],
        issues: vec![],
        issue_count: 0,
        input_fingerprint: None,
        created_at: s.clock.now(),
        finished_at: None,
        error: None,
    };
    tx.execute("INSERT INTO portfolio_history_jobs(id,request_id,portfolio_id,input,key,payload,status) VALUES(?1,?2,?3,?4,?5,?6,'running')",params![result.id,r.request_id,r.portfolio_id,input,bound(key,16384)?,bound(&result,512*1024)?]).map_err(storage)?;
    tx.commit().map_err(storage)?;
    Ok((result, true))
}
pub(super) fn save(
    s: &mut SqliteApplicationStore,
    id: &str,
    offset: usize,
    rows: &[PerformancePoint],
) -> Result<()> {
    if rows.is_empty() || rows.len() > 200 || offset > 100_000 || rows.len() > 100_000 - offset {
        return Err(invalid("history staging page exceeds bounds"));
    }
    bound(&rows, s.limits.max_read_page_bytes)?;
    let tx = transaction(s)?;
    let (status, count): (String, i64) = tx
        .query_row(
            "SELECT status,ordinal_count FROM portfolio_history_jobs WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(storage)?;
    if status != "running" || count != offset as i64 {
        return Err(conflict("history staging is inactive or out of order"));
    }
    let mut previous:Option<String>=tx.query_row("SELECT date FROM portfolio_performance_days WHERE result_id=?1 ORDER BY ordinal DESC LIMIT 1",[id],|r|r.get(0)).optional().map_err(storage)?;
    for (i, row) in rows.iter().enumerate() {
        if let Some(prev) = previous {
            let day: Day = prev.parse().map_err(storage)?;
            if day.succ_opt() != Some(row.date) {
                return Err(invalid("history rows are not consecutive"));
            }
        }
        previous = Some(row.date.to_string());
        tx.execute("INSERT INTO portfolio_performance_days(result_id,ordinal,date,payload) VALUES(?1,?2,?3,?4)",params![id,(offset+i) as i64,row.date.to_string(),bound(row,s.limits.max_read_page_bytes)?]).map_err(storage)?;
    }
    tx.execute(
        "UPDATE portfolio_history_jobs SET ordinal_count=?1 WHERE id=?2",
        params![(offset + rows.len()) as i64, id],
    )
    .map_err(storage)?;
    tx.commit().map_err(storage)
}
pub(super) fn finish(
    s: &mut SqliteApplicationStore,
    r: &PortfolioHistoryResult,
) -> Result<PortfolioHistoryResult> {
    let tx = transaction(s)?;
    let old = result(&tx, &r.key.portfolio_id, &r.id)?;
    if old.status != HistoryStatus::Running {
        return Ok(old);
    }
    if old.key != r.key
        || r.status == HistoryStatus::Running
        || r.status == HistoryStatus::InterruptedOrExternal
    {
        return Err(conflict("history publication does not match running job"));
    }
    let doc = read(&tx, &r.key.portfolio_id)?;
    let mut saved = r.clone();
    if doc.header.revision != r.key.revision
        || history_bindings(&doc)? != r.key.bindings_fingerprint
    {
        saved.status = HistoryStatus::StaleRevision;
    }
    if saved.evidence.len() > 101 || saved.issues.len() > 20 {
        return Err(invalid("history result metadata exceeds bounds"));
    }
    saved.error = saved.error.map(|s| s.chars().take(2048).collect());
    saved.finished_at = Some(s.clock.now());
    if saved.status.published() {
        let (count,first,last):(i64,Option<String>,Option<String>)=tx.query_row("SELECT count(*),min(date),max(date) FROM portfolio_performance_days WHERE result_id=?1",[&r.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(storage)?;
        if count != r.row_count as i64
            || count == 0
            || first != r.baseline.map(|v| v.to_string())
            || last != r.effective_end.map(|v| v.to_string())
        {
            return Err(invalid("history publication has incomplete staged rows"));
        }
    }
    for (i, evidence) in saved.evidence.iter().enumerate() {
        tx.execute(
            "INSERT INTO portfolio_history_evidence(result_id,ordinal,payload) VALUES(?1,?2,?3)",
            params![r.id, i as i64, bound(evidence, 16384)?],
        )
        .map_err(storage)?;
    }
    update(&tx, &saved)?;
    tx.commit().map_err(storage)?;
    Ok(saved)
}
pub(super) fn read_result(
    s: &SqliteApplicationStore,
    p: &str,
    id: &str,
) -> Result<PortfolioHistoryResult> {
    result(&s.connection, p, id)
}
pub(super) fn latest(
    s: &SqliteApplicationStore,
    p: &str,
    account: Option<&str>,
    range: &HistoryRange,
) -> Result<Option<PortfolioHistoryResult>> {
    read(&s.connection, p)?;
    let payload:Option<String>=s.connection.query_row("SELECT payload FROM portfolio_history_jobs WHERE portfolio_id=?1 AND status IN ('complete','partial') AND json_extract(key,'$.account_id') IS ?2 AND json_extract(key,'$.requested_range.start') IS ?3 AND json_extract(key,'$.requested_range.end')=?4 ORDER BY rowid DESC LIMIT 1",params![p,account,range.start.map(|d|d.to_string()),range.end.to_string()],|r|r.get(0)).optional().map_err(storage)?;
    payload
        .map(|p| serde_json::from_str(&p).map_err(storage))
        .transpose()
}
fn rows<T: serde::de::DeserializeOwned>(
    s: &SqliteApplicationStore,
    table: &str,
    id: &str,
    page: PageRequest,
) -> Result<Vec<T>> {
    validate_page(page)?;
    if page.limit > s.limits.max_read_page_items {
        return Err(invalid("history read exceeds configured item limit"));
    }
    let tx = s.connection.unchecked_transaction().map_err(storage)?;
    let size:i64=tx.query_row(&format!("SELECT coalesce(sum(n),0) FROM (SELECT length(CAST(payload AS BLOB)) AS n FROM {table} WHERE result_id=?1 AND ordinal>=?2 ORDER BY ordinal LIMIT ?3)"),params![id,page.offset as i64,page.limit as i64],|r|r.get(0)).map_err(storage)?;
    if size < 0 || size as usize > s.limits.max_read_page_bytes {
        return Err(super::super::limit());
    }
    let mut stmt=tx.prepare(&format!("SELECT payload FROM {table} WHERE result_id=?1 AND ordinal>=?2 ORDER BY ordinal LIMIT ?3")).map_err(storage)?;
    let items = stmt
        .query_map(params![id, page.offset as i64, page.limit as i64], |r| {
            r.get::<_, String>(0)
        })
        .map_err(storage)?
        .map(|r| serde_json::from_str(&r.map_err(storage)?).map_err(storage))
        .collect::<Result<Vec<T>>>()?;
    drop(stmt);
    tx.commit().map_err(storage)?;
    Ok(items)
}
pub(super) fn read_page(
    s: &SqliteApplicationStore,
    p: &str,
    id: &str,
    page: PageRequest,
) -> Result<PortfolioHistoryPage> {
    let result = result(&s.connection, p, id)?;
    if !result.status.published() {
        return Err(conflict("history rows are not published"));
    }
    let items = rows(s, "portfolio_performance_days", id, page)?;
    Ok(PortfolioHistoryPage {
        result_id: id.into(),
        key: result.key,
        next_offset: (page.offset + items.len() < result.row_count)
            .then_some(page.offset + items.len()),
        items,
    })
}
pub(super) fn evidence(
    s: &SqliteApplicationStore,
    p: &str,
    id: &str,
    page: PageRequest,
) -> Result<PortfolioPage<HistoryEvidenceRef>> {
    let result = result(&s.connection, p, id)?;
    if !result.status.published() {
        return Err(conflict("history evidence is not published"));
    }
    let items = rows(s, "portfolio_history_evidence", id, page)?;
    Ok(PortfolioPage {
        revision: result.key.revision,
        next_offset: (page.offset + items.len() < result.evidence.len())
            .then_some(page.offset + items.len()),
        items,
    })
}

pub(super) fn request(
    s: &SqliteApplicationStore,
    r: &PortfolioHistoryRequest,
) -> Result<Option<PortfolioHistoryResult>> {
    let old: Option<(String, String)> = s
        .connection
        .query_row(
            "SELECT input,payload FROM portfolio_history_jobs WHERE request_id=?1",
            [&r.request_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    old.map(|(input, payload)| {
        if input != bound(r, 16384)? {
            return Err(conflict("history request ID reused"));
        }
        serde_json::from_str(&payload).map_err(storage)
    })
    .transpose()
}
pub(super) fn cached(
    s: &SqliteApplicationStore,
    p: &str,
) -> Result<Option<PortfolioHistoryResult>> {
    read(&s.connection, p)?;
    let payload:Option<String>=s.connection.query_row("SELECT payload FROM portfolio_history_jobs WHERE portfolio_id=?1 AND status IN ('complete','partial') ORDER BY rowid DESC LIMIT 1",[p],|r|r.get(0)).optional().map_err(storage)?;
    payload
        .map(|v| serde_json::from_str(&v).map_err(storage))
        .transpose()
}
