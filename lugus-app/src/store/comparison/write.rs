use super::*;
impl ComparisonStore for SqliteApplicationStore {
    fn lookup_request(&self, s: &Scope, r: &ComparisonRequest) -> Result<Option<ComparisonJob>> {
        self.comparison_lookup(s, r)
    }
    fn acquire(&self, s: &Scope) -> Result<ComparisonLease> {
        s.validate()?;
        ComparisonLease::acquire(&self.store_key, &s.workspace_id)
    }
    fn job(&self, s: &Scope, id: &str) -> Result<ComparisonJob> {
        self.comparison_header("comparison_jobs", s, id)
    }
    fn read(&self, s: &Scope, id: &str) -> Result<ComparisonRecord> {
        self.comparison_header("comparison_records", s, id)
    }
    fn list(&self, s: &Scope, p: PageRequest) -> Result<ComparisonPage<ComparisonSummary>> {
        self.comparison_list(s, p)
    }
    fn package(&self, s: &Scope, id: &str) -> Result<ResearchPackage> {
        self.comparison_header("research_packages", s, id)
    }
    fn rows(&self, s: &Scope, id: &str, p: PageRequest) -> Result<ComparisonPage<ComparisonRow>> {
        let r = ComparisonStore::read(self, s, id)?;
        self.comparison_page(&r.package_id, "rows", p)
    }
    fn sources(
        &self,
        s: &Scope,
        id: &str,
        p: PageRequest,
    ) -> Result<ComparisonPage<ComparisonSource>> {
        let r = ComparisonStore::read(self, s, id)?;
        self.comparison_page(&r.package_id, "sources", p)
    }
    fn package_entries(
        &self,
        s: &Scope,
        id: &str,
        p: PageRequest,
    ) -> Result<ComparisonPage<ResearchPackageEntry>> {
        let r = self.package(s, id)?;
        self.comparison_page(&r.id, "entries", p)
    }
    fn begin(
        &mut self,
        s: &Scope,
        r: &ComparisonRequest,
        l: &ComparisonLease,
        p: &CapturedProviders,
    ) -> Result<ComparisonJob> {
        s.validate()?;
        r.validate(self.clock.now().date_naive())?;
        if r.request_id != s.request_id || !l.matches(&self.store_key, &s.workspace_id) {
            return Err(conflict());
        }
        if let Some(j) = self.comparison_lookup(s, r)? {
            return Ok(j);
        }
        if let Some(id) = &r.previous_id {
            let prior = ComparisonStore::read(self, s, id)?;
            let mut old = prior.request;
            old.request_id = r.request_id.clone();
            old.previous_id = r.previous_id.clone();
            if serde_json::to_value(old).map_err(storage)?
                != serde_json::to_value(r).map_err(storage)?
            {
                return Err(conflict());
            }
        }
        let job = ComparisonJob {
            id: self.id()?,
            workspace_id: s.workspace_id.clone(),
            repository_id: self.evidence.repository_identity()?,
            request: r.clone(),
            providers: p.clone(),
            state: ComparisonState::Running,
            fetch_ids: vec![],
            comparison_id: None,
            created_at: self.clock.now(),
            finished_at: None,
            error: None,
            owner: ComparisonOwner::None,
        };
        let input = encode(self, r)?;
        let payload = encode(self, &job)?;
        self.connection.execute("INSERT INTO comparison_jobs(id,workspace,repository,request,input,state,payload) VALUES(?1,?2,?3,?4,?5,'running',?6)",params![job.id,s.workspace_id,job.repository_id,r.request_id,input,payload]).map_err(|e|if matches!(e,rusqlite::Error::SqliteFailure(_, _)){conflict()}else{storage(e)})?;
        Ok(job)
    }
    fn record_fetch(&mut self, s: &Scope, id: &str, f: &FetchReference) -> Result<()> {
        let owned = self.read_fetch(s, &f.id)?;
        let mut j = self.job(s, id)?;
        if !j.state.terminal() && !j.fetch_ids.contains(&owned.id) {
            j.fetch_ids.push(owned.id);
            self.save_comparison_job(&j)?;
        }
        Ok(())
    }
    fn finish(
        &mut self,
        s: &Scope,
        id: &str,
        state: ComparisonState,
        error: Option<AppError>,
    ) -> Result<ComparisonJob> {
        if !matches!(
            state,
            ComparisonState::Cancelled | ComparisonState::Failed | ComparisonState::Interrupted
        ) {
            return Err(crate::comparison::invalid("invalid terminal transition"));
        }
        let mut j = self.job(s, id)?;
        if !j.state.terminal() {
            j.state = state;
            j.error = error;
            j.finished_at = Some(self.clock.now());
            j.owner = ComparisonOwner::None;
            self.save_comparison_job(&j)?;
        }
        Ok(j)
    }
    fn recover_interrupted(
        &mut self,
        s: &Scope,
        l: &ComparisonLease,
    ) -> Result<Vec<ComparisonJob>> {
        if !l.matches(&self.store_key, &s.workspace_id) {
            return Err(conflict());
        }
        let repo = self.evidence.repository_identity()?;
        let id:Option<String>=self.connection.query_row("SELECT id FROM comparison_jobs WHERE workspace=?1 AND repository=?2 AND state='running'",params![s.workspace_id,repo],|r|r.get(0)).optional().map_err(storage)?;
        id.map(|id| self.finish(s, &id, ComparisonState::Interrupted, None))
            .transpose()
            .map(|v| v.into_iter().collect())
    }
    fn publish(&mut self, s: &Scope, id: &str, p: &PreparedComparison) -> Result<ComparisonJob> {
        let mut job = self.job(s, id)?;
        if job.state.terminal() {
            return Ok(job);
        }
        if p.companies[0].company == p.companies[1].company
            || p.companies
                .iter()
                .any(|c| !p.dependencies.contains(&c.resolution_dataset_id))
        {
            return Err(scoped());
        }
        for company in &p.companies {
            let header = self.dataset_header(s, &company.resolution_dataset_id)?;
            if !matches!(header.projection, DatasetProjection::Resolution { .. }) {
                return Err(scoped());
            }
        }
        for dataset in &p.dependencies {
            self.dataset_header(s, dataset)?;
        }
        let package_id = self.id()?;
        let comparison_id = self.id()?;
        let now = self.clock.now();
        let package = ResearchPackage {
            id: package_id.clone(),
            workspace_id: s.workspace_id.clone(),
            repository_id: job.repository_id.clone(),
            schema_version: 1,
            policy: "annual-comparison:1".into(),
            created_at: now,
            companies: p.companies.clone(),
            dependency_count: p.dependencies.len(),
            row_count: p.rows.len(),
            source_count: p.sources.len(),
            entry_count: p.entries.len(),
            fingerprint: lugus_financial::domain::fingerprint(&(&p.rows, &p.sources, &p.entries))?,
        };
        let state = if p.partial {
            ComparisonState::Partial
        } else {
            ComparisonState::Complete
        };
        let record = ComparisonRecord {
            id: comparison_id.clone(),
            package_id: package_id.clone(),
            request: job.request.clone(),
            companies: p.companies.clone(),
            row_count: p.rows.len(),
            source_count: p.sources.len(),
            previous_id: job.request.previous_id.clone(),
            state,
            created_at: now,
            issues: p.issues.clone(),
        };
        let package_json = encode(self, &package)?;
        let record_json = encode(self, &record)?;
        let mut entries = vec![];
        let mut total = package_json.len().saturating_add(record_json.len());
        for (section, values) in [
            (
                "rows",
                p.rows
                    .iter()
                    .map(|v| encode(self, v))
                    .collect::<Result<Vec<_>>>()?,
            ),
            (
                "sources",
                p.sources
                    .iter()
                    .map(|v| encode(self, v))
                    .collect::<Result<Vec<_>>>()?,
            ),
            (
                "entries",
                p.entries
                    .iter()
                    .map(|v| encode(self, v))
                    .collect::<Result<Vec<_>>>()?,
            ),
        ] {
            for (i, raw) in values.into_iter().enumerate() {
                total = total.saturating_add(raw.len());
                if total > self.limits.max_bytes_per_fetch {
                    return Err(limit());
                }
                entries.push((section, i, raw));
            }
        }
        job.state = state;
        job.comparison_id = Some(comparison_id.clone());
        job.finished_at = Some(now);
        job.owner = ComparisonOwner::None;
        let job_json = encode(self, &job)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage)?;
        tx.execute(
            "INSERT INTO research_packages VALUES(?1,?2,?3,?4)",
            params![package_id, s.workspace_id, job.repository_id, package_json],
        )
        .map_err(storage)?;
        tx.execute(
            "INSERT INTO comparison_records VALUES(?1,?2,?3,?4,?5)",
            params![
                comparison_id,
                s.workspace_id,
                job.repository_id,
                package_id,
                record_json
            ],
        )
        .map_err(storage)?;
        for (section, i, raw) in entries {
            tx.execute(
                "INSERT INTO comparison_entries VALUES(?1,?2,?3,?4)",
                params![package_id, section, i as i64, raw],
            )
            .map_err(storage)?;
        }
        for dep in &p.dependencies {
            tx.execute(
                "INSERT INTO comparison_dependencies VALUES(?1,?2)",
                params![package_id, dep],
            )
            .map_err(storage)?;
        }
        tx.execute(
            "UPDATE comparison_jobs SET state=?2,payload=?3 WHERE id=?1 AND state='running'",
            params![id, if p.partial { "partial" } else { "complete" }, job_json],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(job)
    }
}
