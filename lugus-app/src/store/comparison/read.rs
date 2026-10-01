use super::*;
impl SqliteApplicationStore {
    pub(super) fn comparison_lookup(
        &self,
        s: &Scope,
        r: &ComparisonRequest,
    ) -> Result<Option<ComparisonJob>> {
        s.validate()?;
        let repo = self.evidence.repository_identity()?;
        let id:Option<String>=self.connection.query_row("SELECT id FROM comparison_jobs WHERE workspace=?1 AND repository=?2 AND request=?3",params![s.workspace_id,repo,r.request_id],|r|r.get(0)).optional().map_err(storage)?;
        let Some(id) = id else { return Ok(None) };
        let job: ComparisonJob = self.comparison_header("comparison_jobs", s, &id)?;
        if serde_json::to_value(&job.request).map_err(storage)?
            != serde_json::to_value(r).map_err(storage)?
        {
            return Err(conflict());
        }
        Ok(Some(job))
    }
    pub(super) fn comparison_list(
        &self,
        s: &Scope,
        p: PageRequest,
    ) -> Result<ComparisonPage<ComparisonSummary>> {
        s.validate()?;
        if p.limit == 0 || p.limit > self.limits.max_read_page_items || p.offset > i64::MAX as usize
        {
            return Err(limit());
        }
        let repo = self.evidence.repository_identity()?;
        let mut q=self.connection.prepare("SELECT id FROM comparison_records WHERE workspace=?1 AND repository=?2 ORDER BY rowid DESC LIMIT ?3 OFFSET ?4").map_err(storage)?;
        let ids = q
            .query_map(
                params![
                    s.workspace_id,
                    repo,
                    p.limit.saturating_add(1).min(i64::MAX as usize) as i64,
                    p.offset as i64
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(storage)?;
        let mut items = vec![];
        let mut next = None;
        let mut bytes = 128usize;
        for id in ids {
            if items.len() == p.limit {
                next = Some(p.offset + items.len());
                break;
            }
            let value: ComparisonRecord =
                self.comparison_header("comparison_records", s, &id.map_err(storage)?)?;
            let n = serde_json::to_vec(&value).map_err(storage)?.len();
            if bytes.saturating_add(n) > self.limits.max_output_bytes {
                if items.is_empty() {
                    return Err(limit());
                }
                next = Some(p.offset + items.len());
                break;
            }
            bytes += n + 1;
            items.push(value);
        }
        Ok(ComparisonPage {
            items,
            next_offset: next,
        })
    }
}
