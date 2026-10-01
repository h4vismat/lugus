use super::*;
use crate::research::{resolution_input, resolve_candidate};
use lugus_financial::{
    comparison::{ComparisonIssue, FactInput, calculate_annual, select_annual},
    domain::CompanyId,
};
struct Work<'a> {
    app: &'a Application,
    scope: &'a Scope,
    job: &'a ComparisonJob,
    offering: &'a Offering,
    stop: &'a ComparisonStop,
    next: usize,
    entries: Vec<ResearchPackageEntry>,
    dependencies: Vec<String>,
    issues: Vec<ComparisonIssue>,
    bytes: usize,
    count: usize,
}
struct Child {
    app: Application,
    scope: Scope,
    id: Option<String>,
}
impl Drop for Child {
    fn drop(&mut self) {
        if let Some(id) = &self.id {
            let _ = self.app.cancel(&self.scope, id);
        }
    }
}
impl Work<'_> {
    fn issue(&mut self, code: &str, detail: &str) {
        self.issues.push(ComparisonIssue::new(code, detail, vec![]));
    }
    fn scoped(&mut self) -> Scope {
        self.next += 1;
        Scope {
            workspace_id: self.scope.workspace_id.clone(),
            request_id: format!("{}:{}", self.job.id, self.next),
            run_id: None,
        }
    }
    async fn fetch(&mut self, command: FetchCommand) -> Result<FetchReference> {
        self.stop.check()?;
        let sc = self.scoped();
        let receipt = self.app.submit(&sc, self.offering, command)?;
        let mut guard = Child {
            app: self.app.clone(),
            scope: sc.clone(),
            id: Some(receipt.id.clone()),
        };
        let mut cancel = self.stop.cancel.clone();
        let result = tokio::select! {biased;_=async{while !*cancel.borrow_and_update(){if cancel.changed().await.is_err(){break;}}}=>Err(error(ErrorKind::Cancelled,"Comparison cancelled")),_=self.stop.deadline.wait()=>Err(error(ErrorKind::Timeout,"Comparison deadline exceeded")),result=self.app.wait(&sc,&receipt.id)=>result};
        let status = match result {
            Ok(s) => s,
            Err(e) => {
                let _ = self.app.cancel(&sc, &receipt.id);
                if let Ok(status) = self.app.wait(&sc, &receipt.id).await {
                    self.save_fetch(&sc, &status).await?;
                }
                guard.id = None;
                return Err(e);
            }
        };
        guard.id = None;
        self.save_fetch(&sc, &status).await?.ok_or_else(|| {
            status
                .error
                .unwrap_or_else(|| error(ErrorKind::MissingData, "Fetch returned no receipt"))
        })
    }
    async fn save_fetch(
        &mut self,
        sc: &Scope,
        status: &JobStatus,
    ) -> Result<Option<FetchReference>> {
        let Some(id) = &status.fetch_id else {
            return Ok(None);
        };
        let f = self.app.read_fetch(sc, id).await?;
        let (s, j, copy) = (self.scope.clone(), self.job.id.clone(), f.clone());
        self.app
            .comparison_effect(move |store| store.record_fetch(&s, &j, &copy))
            .await?;
        self.entries.push(ResearchPackageEntry::Fetch(f.clone()));
        Ok(Some(f))
    }
    async fn dataset(
        &mut self,
        f: &FetchReference,
        projection: DatasetProjection,
    ) -> Result<(DatasetHeader, Vec<DatasetRow>)> {
        self.stop.check()?;
        let sc = self.scoped();
        let header = self.app.create_dataset(&sc, &f.id, projection).await?;
        self.dependencies.push(header.id.clone());
        self.entries
            .push(ResearchPackageEntry::Dependency(header.clone()));
        let mut rows = vec![];
        let mut offset = 0;
        loop {
            self.stop.check()?;
            let mut n = self.app.limits().max_read_page_items.min(100);
            let page = loop {
                match self
                    .app
                    .read_dataset(&sc, &header.id, PageRequest { offset, limit: n })
                    .await
                {
                    Err(e) if e.kind == ErrorKind::ResourceLimit && n > 1 => n /= 2,
                    v => break v?,
                }
            };
            self.bytes = self.bytes.saturating_add(
                serde_json::to_vec(&page)
                    .map_err(|_| error(ErrorKind::InvalidInput, "Evidence serialization failed"))?
                    .len(),
            );
            self.count = self.count.saturating_add(page.rows.len());
            if self.bytes > self.app.limits().max_bytes_per_fetch
                || self.count > self.app.limits().max_items_per_fetch
            {
                return Err(error(
                    ErrorKind::ResourceLimit,
                    "Comparison evidence exceeds configured limits",
                ));
            }
            rows.extend(page.rows);
            match page.next_offset {
                Some(next) if next > offset => offset = next,
                Some(_) => {
                    return Err(error(
                        ErrorKind::Storage,
                        "Evidence pagination did not advance",
                    ));
                }
                None => break,
            }
        }
        Ok((header, rows))
    }
    async fn resolve(&mut self, index: usize) -> Result<ResolvedCompany> {
        let mention = &self.job.request.subjects[index];
        let f = self
            .fetch(FetchCommand::Resolve {
                instance_id: self.job.providers.resolution.instance_id.clone(),
                input: resolution_input(&mention.text)?,
            })
            .await?;
        if let Some(e) = f.error {
            return Err(e);
        }
        let id = run_id(&f, RunKind::Resolution)?;
        let (header, rows) = self
            .dataset(&f, DatasetProjection::Resolution { run_id: id })
            .await?;
        let company = resolve_candidate(header.resolution_status.as_deref(), &rows, None)?;
        if mention.exchange.as_ref().is_some_and(|exchange| {
            !company.candidate.listings.iter().any(|l| {
                l.exchange
                    .as_ref()
                    .is_some_and(|e| e.value.eq_ignore_ascii_case(exchange))
            })
        }) {
            return Err(error(
                ErrorKind::NeedsAttention,
                "Exchange does not match the resolved company",
            ));
        }
        Ok(ResolvedCompany {
            company: company.candidate.identifier,
            name: company.candidate.name,
            resolution_dataset_id: header.id,
        })
    }
    async fn facts(&mut self, company: &CompanyId) -> Result<(String, Vec<FactInput>)> {
        let today = self.job.created_at.date_naive();
        let query = Query {
            company: company.clone(),
            filed_from: today
                .checked_sub_months(chrono::Months::new(120))
                .ok_or_else(|| error(ErrorKind::InvalidInput, "Invalid comparison date"))?,
            filed_to: today,
            forms: vec!["10-K".into(), "10-K/A".into()],
            page_size: 100,
            cursor: None,
        };
        let f = self
            .fetch(FetchCommand::Facts {
                instance_id: self.job.providers.facts.instance_id.clone(),
                query,
            })
            .await?;
        if let Some(e) = &f.error {
            self.issue("source_failure", &e.message);
        }
        let id = run_id(&f, RunKind::Financial)?;
        let (header, rows) = self
            .dataset(&f, DatasetProjection::AllFacts { run_id: id })
            .await?;
        let mut facts = vec![];
        for (ordinal, row) in rows.into_iter().enumerate() {
            match row {
                DatasetRow::ReportedFact { evidence } => facts.push(FactInput {
                    dataset_id: header.id.clone(),
                    ordinal,
                    evidence,
                }),
                _ => {
                    return Err(error(
                        ErrorKind::Storage,
                        "Unexpected financial evidence row",
                    ));
                }
            }
        }
        Ok((header.id, facts))
    }
}
fn run_id(f: &FetchReference, kind: RunKind) -> Result<i64> {
    f.runs
        .iter()
        .find(|r| r.kind == kind)
        .map(|r| r.id)
        .ok_or_else(|| error(ErrorKind::MissingData, "Fetch has no compatible source run"))
}
pub(super) async fn prepare_comparison(
    app: &Application,
    scope: &Scope,
    job: &ComparisonJob,
    offering: &Offering,
    stop: &ComparisonStop,
) -> Result<PreparedComparison> {
    let mut w = Work {
        app,
        scope,
        job,
        offering,
        stop,
        next: 0,
        entries: vec![],
        dependencies: vec![],
        issues: vec![],
        bytes: 0,
        count: 0,
    };
    let companies = if let Some(id) = &job.request.previous_id {
        let previous = app.read_comparison(scope, id).await?;
        for c in &previous.companies {
            let h = app.dataset_header(scope, &c.resolution_dataset_id).await?;
            w.dependencies.push(h.id.clone());
            w.entries.push(ResearchPackageEntry::Dependency(h));
        }
        previous.companies
    } else {
        [w.resolve(0).await?, w.resolve(1).await?]
    };
    if companies[0].company == companies[1].company {
        return Err(error(
            ErrorKind::NeedsAttention,
            "Both subjects resolve to the same company",
        ));
    }
    let mut rows = vec![];
    let mut sources = vec![];
    let mut partial = false;
    for company in &companies {
        stop.check()?;
        let facts = match w.facts(&company.company).await {
            Ok((_, f)) => f,
            Err(e)
                if matches!(
                    e.kind,
                    ErrorKind::Cancelled | ErrorKind::Timeout | ErrorKind::ResourceLimit
                ) =>
            {
                return Err(e);
            }
            Err(e) => {
                partial = true;
                w.issue("source_failure", &e.message);
                vec![]
            }
        };
        let selection = select_annual(&company.company, &facts, &job.request.policy())?;
        let calculated = calculate_annual(&selection, &job.request.policy())?;
        if calculated.len() != job.request.years as usize
            || calculated
                .iter()
                .any(|r| r.net_margin.result.is_none() || r.revenue_growth.result.is_none())
        {
            partial = true;
        }
        for issue in &selection.issues {
            w.entries
                .push(ResearchPackageEntry::Limitation(issue.clone()));
            w.issue(&issue.code, &issue.detail);
        }
        let refs: std::collections::BTreeSet<_> = selection
            .periods
            .iter()
            .flat_map(|p| p.revenue.inputs.iter().chain(&p.net_income.inputs))
            .collect();
        for f in &facts {
            let input = f.reference();
            if refs.contains(&input) {
                sources.push(ComparisonSource {
                    company: company.company.clone(),
                    metric: f.evidence.value.concept.clone(),
                    input,
                    fact: f.evidence.value.clone(),
                });
            }
        }
        w.entries.push(ResearchPackageEntry::Selection(selection));
        for annual in calculated {
            let r = ComparisonRow {
                company: company.company.clone(),
                annual,
            };
            w.entries.push(ResearchPackageEntry::Calculation(r.clone()));
            rows.push(r);
        }
    }
    if w.issues.iter().any(|i| i.code == "source_failure") {
        partial = true;
    }
    w.issue(
        "coverage_unverified",
        "Source completeness and accounting-basis comparability have not been verified",
    );
    if let (Some(a), Some(b)) = (
        rows.iter()
            .rev()
            .find(|r| r.company == companies[0].company),
        rows.iter()
            .rev()
            .find(|r| r.company == companies[1].company),
    ) && a.annual.period.end != b.annual.period.end
    {
        w.issue(
            "different_fiscal_calendars",
            "Companies have different annual reporting endpoints",
        );
    }
    for issue in &w.issues {
        w.entries
            .push(ResearchPackageEntry::Limitation(issue.clone()));
    }
    w.dependencies.sort();
    w.dependencies.dedup();
    w.issues.sort_by(|a, b| a.code.cmp(&b.code));
    w.issues
        .dedup_by(|a, b| a.code == b.code && a.detail == b.detail);
    Ok(PreparedComparison {
        companies,
        dependencies: w.dependencies,
        rows,
        sources,
        entries: w.entries,
        issues: w.issues,
        partial,
    })
}
