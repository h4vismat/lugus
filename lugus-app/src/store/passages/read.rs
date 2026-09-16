use super::*;
use serde::de::DeserializeOwned;
pub(super) fn record<T: DeserializeOwned>(
    db: &Connection,
    scope: &Scope,
    repository: &str,
    id: &str,
    category: &str,
    max: usize,
) -> Result<T> {
    scope.validate()?;
    super::super::sqlite::validate_id(id)?;
    let meta:Option<(bool,bool,bool,i64)>=db.query_row("SELECT workspace=?2,repository=?3,category=?4,length(CAST(payload AS BLOB)) FROM app_records WHERE id=?1",params![id,scope.workspace_id,repository,category],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(storage)?;
    let (workspace, repo, kind, len) =
        meta.ok_or_else(|| error(ErrorKind::MissingData, "text reference unavailable"))?;
    if !workspace || !repo {
        return Err(error(
            ErrorKind::ScopeMismatch,
            "text reference belongs to another scope",
        ));
    }
    if !kind {
        return Err(invalid());
    }
    bounded(len, max)?;
    let payload: String = db
        .query_row("SELECT payload FROM app_records WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .map_err(storage)?;
    if category == "text" || category == "passage" {
        let sql = if category == "text" {
            "SELECT checksum FROM text_representations WHERE id=?1 AND length(CAST(checksum AS BLOB))=64"
        } else {
            "SELECT checksum FROM passage_requests WHERE passage=?1 AND length(CAST(checksum AS BLOB))=64"
        };
        let checksum: String = db.query_row(sql, [id], |r| r.get(0)).map_err(storage)?;
        if checksum != text_checksum(&payload) {
            return Err(corrupt());
        }
    }
    serde_json::from_str(&payload).map_err(storage)
}
pub(super) fn dataset(
    db: &Connection,
    scope: &Scope,
    repo: &str,
    id: &str,
    max: usize,
) -> Result<DatasetHeader> {
    let h: DatasetHeader = record(db, scope, repo, id, "dataset", max)?;
    if h.id != id
        || h.workspace_id != scope.workspace_id
        || h.repository_id != repo
        || h.kind != DatasetKind::Document
        || !matches!(h.projection, DatasetProjection::Document)
        || h.error.is_some()
        || !h
            .document
            .as_ref()
            .is_some_and(|d| d.provider == h.provider)
    {
        return Err(corrupt());
    }
    Ok(h)
}
pub(super) fn header(
    db: &Connection,
    scope: &Scope,
    repo: &str,
    id: &str,
    max: usize,
    unlimited: bool,
) -> Result<TextRepresentation> {
    let h: TextRepresentation = record(
        db,
        scope,
        repo,
        id,
        "text",
        if unlimited { max } else { max.min(HEADER_MAX) },
    )?;
    let cap = TextLimits::for_research_mode(unlimited);
    if h.id != id
        || h.workspace_id != scope.workspace_id
        || h.repository_id != repo
        || h.text_bytes > cap.max_text_bytes
        || h.mapping_count > cap.max_mappings
        || h.source_node_count > cap.max_nodes
        || h.text_checksum.len() != 64
    {
        return Err(corrupt());
    }
    check_envelope(&h, max)?;
    Ok(h)
}
pub(super) fn chunks(
    db: &Connection,
    id: &str,
    node: i64,
    start: usize,
    end: usize,
    total: usize,
    max: usize,
) -> Result<String> {
    let length = end.checked_sub(start).ok_or_else(invalid)?;
    if end > total {
        return Err(invalid());
    }
    if length > max {
        return Err(limit());
    }
    let stored_end: i64 = db
        .query_row(
            "SELECT coalesce(max(end),0) FROM text_chunks WHERE representation=?1 AND node=?2",
            params![id, node],
            |r| r.get(0),
        )
        .map_err(storage)?;
    if bounded(stored_end, total)? != total {
        return Err(corrupt());
    }
    if length == 0 && start == total {
        return Ok(String::new());
    }
    let query_end = end.max(start + 1);
    let (count,bytes,bad):(i64,i64,i64)=db.query_row("SELECT count(*),coalesce(sum(length(CAST(text AS BLOB))),0),coalesce(sum(start<0 OR end<=start OR end-start!=length(CAST(text AS BLOB)) OR length(CAST(text AS BLOB))>4096 OR length(CAST(checksum AS BLOB))!=64),0) FROM text_chunks WHERE representation=?1 AND node=?2 AND end>?3 AND start<?4",params![id,node,start as i64,query_end as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(storage)?;
    bounded(count, length / (CHUNK - 3) + 2)?;
    bounded(bytes, length.saturating_add(2 * CHUNK))?;
    if bad != 0 || count == 0 {
        return Err(corrupt());
    }
    let mut stmt=db.prepare("SELECT start,end,text,checksum FROM text_chunks WHERE representation=?1 AND node=?2 AND end>?3 AND start<?4 ORDER BY start").map_err(storage)?;
    let mut rows = stmt
        .query(params![id, node, start as i64, query_end as i64])
        .map_err(storage)?;
    let mut result = String::with_capacity(length);
    let mut covered = start;
    while let Some(row) = rows.next().map_err(storage)? {
        let a = bounded(row.get(0).map_err(storage)?, total)?;
        let b = bounded(row.get(1).map_err(storage)?, total)?;
        let text: String = row.get(2).map_err(storage)?;
        let checksum: String = row.get(3).map_err(storage)?;
        if a > covered
            || (covered != start && a != covered)
            || b <= covered
            || checksum != text_checksum(&text)
        {
            return Err(corrupt());
        }
        let slice = text.get(covered - a..end.min(b) - a).ok_or_else(invalid)?;
        result.push_str(slice);
        covered = end.min(b);
    }
    if covered != end || result.len() != length {
        return Err(corrupt());
    }
    Ok(result)
}
pub(super) fn mappings(
    db: &Connection,
    h: &TextRepresentation,
    start: usize,
    end: usize,
    max: usize,
) -> Result<Vec<SourceMapping>> {
    // The scoped, checksummed header bounds this covering-index inventory scan.
    // Canonical coverage alone cannot detect a deleted overlapping contributor.
    // No mapping payload is loaded here; even corrupt excess rows stop at cap + 1.
    let expected = h.mapping_count as i64;
    let inventory: (i64, i64, i64) = db.query_row(
        "SELECT count(*),coalesce(min(ordinal),0),coalesce(max(ordinal),-1) FROM (SELECT ordinal FROM text_mappings WHERE representation=?1 ORDER BY ordinal LIMIT ?2)",
        params![h.id, expected.saturating_add(1)],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).map_err(storage)?;
    if inventory != (expected, 0, expected - 1) {
        return Err(corrupt());
    }
    let (count,bytes,bad):(i64,i64,i64)=db.query_row("SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))),0),coalesce(sum(start<0 OR end<=start OR ordinal<0 OR ordinal>=?4 OR length(CAST(checksum AS BLOB))!=64 OR length(CAST(payload AS BLOB))>4096),0) FROM text_mappings WHERE representation=?1 AND end>?2 AND start<?3",params![h.id,start as i64,end as i64,h.mapping_count as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(storage)?;
    bounded(count, h.mapping_count.min(max))?;
    bounded(bytes, max)?;
    if bad != 0 {
        return Err(corrupt());
    }
    let mut stmt=db.prepare("SELECT start,end,payload,checksum FROM text_mappings WHERE representation=?1 AND end>?2 AND start<?3 ORDER BY ordinal").map_err(storage)?;
    let rows = stmt
        .query_map(params![h.id, start as i64, end as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(storage)?;
    let mut result = Vec::new();
    let mut previous: Option<SourceMapping> = None;
    for row in rows {
        let (a, b, payload, sum) = row.map_err(storage)?;
        if text_checksum(&payload) != sum {
            return Err(corrupt());
        }
        let m: SourceMapping = serde_json::from_str(&payload).map_err(storage)?;
        if m.start != bounded(a, h.text_bytes)? || m.end != bounded(b, h.text_bytes)? {
            return Err(corrupt());
        }
        if let Some(p) = &previous
            && m.start < p.end
            && !(m.start == p.start
                && m.end == p.end
                && m.kind == MappingKind::Normalized
                && p.kind == MappingKind::Normalized)
        {
            return Err(corrupt());
        }
        previous = Some(m.clone());
        result.push(m);
    }
    // A valid selection of structural separators is a caller error. Every other
    // clipping failure denotes damaged stored mappings, not an invalid request.
    if result.first().is_some_and(|m| m.start <= start)
        && result.last().is_some_and(|m| m.end >= end)
        && result
            .iter()
            .all(|m| m.kind == MappingKind::Synthetic && m.source.is_none() && m.end - m.start == 1)
        && result.windows(2).all(|pair| pair[0].end == pair[1].start)
    {
        return Err(invalid());
    }
    clip_mappings(&result, start, end).map_err(|_| corrupt())
}
pub(super) fn passage(
    db: &Connection,
    scope: &Scope,
    repo: &str,
    id: &str,
    max: usize,
    unlimited: bool,
) -> Result<Passage> {
    let p: Passage = record(db, scope, repo, id, "passage", max)?;
    if p.id != id
        || p.scope.workspace_id != scope.workspace_id
        || p.quote.len() > TextLimits::for_research_mode(unlimited).max_passage_bytes
        || p.quote_checksum != text_checksum(&p.quote)
    {
        return Err(corrupt());
    }
    let h = header(db, scope, repo, &p.representation.id, max, unlimited)?;
    if serde_json::to_value(&h).map_err(storage)?
        != serde_json::to_value(&p.representation).map_err(storage)?
    {
        return Err(corrupt());
    }
    let text = chunks(
        db,
        &h.id,
        -1,
        p.start,
        p.end,
        h.text_bytes,
        TextLimits::for_research_mode(unlimited).max_passage_bytes,
    )?;
    let current_mappings = mappings(db, &h, p.start, p.end, max).map_err(|e| {
        if e.kind == ErrorKind::InvalidInput {
            corrupt()
        } else {
            e
        }
    })?;
    if text != p.quote || current_mappings != p.mappings {
        return Err(corrupt());
    }
    check_envelope(&p, max)?;
    Ok(p)
}
pub(super) fn sources(
    db: &Connection,
    p: &Passage,
    max: usize,
    unlimited: bool,
) -> Result<Vec<SourceExcerpt>> {
    let mut result = Vec::new();
    let mut used = 2;
    if used > max {
        return Err(limit());
    }
    for mapping in &p.mappings {
        if let Some(source) = &mapping.source {
            let (size,path_size,sum_size):(i64,i64,i64)=db.query_row("SELECT bytes,length(CAST(path AS BLOB)),length(CAST(checksum AS BLOB)) FROM text_nodes WHERE representation=?1 AND node=?2",params![p.representation.id,i64::from(source.node_id)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(storage)?;
            let total = bounded(
                size,
                TextLimits::for_research_mode(unlimited).max_source_bytes,
            )?;
            bounded(
                path_size,
                if unlimited {
                    max.saturating_sub(used)
                } else {
                    HEADER_MAX.min(max.saturating_sub(used))
                },
            )?;
            if sum_size != 64 {
                return Err(corrupt());
            }
            let (path, sum): (String, String) = db
                .query_row(
                    "SELECT path,checksum FROM text_nodes WHERE representation=?1 AND node=?2",
                    params![p.representation.id, i64::from(source.node_id)],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(storage)?;
            if text_checksum(&path) != sum {
                return Err(corrupt());
            }
            let path: Vec<u32> = serde_json::from_str(&path).map_err(storage)?;
            if path.is_empty() || path.len() > TextLimits::for_research_mode(unlimited).max_depth {
                return Err(corrupt());
            }
            let text = chunks(
                db,
                &p.representation.id,
                i64::from(source.node_id),
                source.start,
                source.end,
                total,
                max.saturating_sub(used),
            )?;
            let canonical = p
                .quote
                .get(
                    mapping.start.checked_sub(p.start).ok_or_else(corrupt)?
                        ..mapping.end.checked_sub(p.start).ok_or_else(corrupt)?,
                )
                .ok_or_else(corrupt)?;
            if (mapping.kind == MappingKind::Exact && text != canonical)
                || (mapping.kind == MappingKind::Normalized
                    && (canonical != " " || !text.chars().all(char::is_whitespace)))
            {
                return Err(corrupt());
            }
            let excerpt = SourceExcerpt {
                node_id: source.node_id,
                path,
                start: source.start,
                end: source.end,
                text,
            };
            used = used
                .checked_add(check_envelope(&excerpt, max.saturating_sub(used))?)
                .and_then(|n| n.checked_add(usize::from(!result.is_empty())))
                .filter(|n| *n <= max)
                .ok_or_else(limit)?;
            result.push(excerpt);
        }
    }
    Ok(result)
}
