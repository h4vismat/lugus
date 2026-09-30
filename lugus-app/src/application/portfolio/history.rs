use super::history_input::history_inputs;
use super::*;
use chrono_tz::America::New_York;
use lugus_financial::{
    domain::fingerprint, historical_prices::HistoryQuery, storage::history::HistoryReadPage,
};
use lugus_portfolio::{
    Day, EventKind, HistoryIssue, HistoryIssueCode, Opening, ValuationHistoryInput,
    calculate_performance_cancellable, historical_values_cancellable,
};
use std::collections::BTreeSet;
struct ChildFetch {
    app: Application,
    scope: Scope,
    id: String,
}
impl Drop for ChildFetch {
    fn drop(&mut self) {
        let _ = self.app.cancel(&self.scope, &self.id);
    }
}
#[derive(Clone)]
struct Stop {
    cancel: watch::Receiver<bool>,
    deadline: lugus_agent::deadline::Deadline,
}
impl Stop {
    fn stopped(&self) -> bool {
        *self.cancel.borrow() || self.deadline.expired()
    }
    fn check(&self) -> Result<()> {
        if *self.cancel.borrow() {
            Err(error(ErrorKind::Cancelled, "portfolio history cancelled"))
        } else if self.deadline.expired() {
            Err(error(
                ErrorKind::Timeout,
                "portfolio history deadline exceeded",
            ))
        } else {
            Ok(())
        }
    }
    async fn wait<T>(&self, future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        self.check()?;
        let mut cancel = self.cancel.clone();
        tokio::select! {biased;
            _=async{while !*cancel.borrow_and_update(){if cancel.changed().await.is_err(){break;}}}=>Err(error(ErrorKind::Cancelled,"portfolio history cancelled")),
            _=self.deadline.wait()=>Err(error(ErrorKind::Timeout,"portfolio history deadline exceeded")),
            result=future=>result,
        }
    }
}
impl Application {
    pub fn history_providers(&self) -> Result<Vec<ProviderIdentity>> {
        Ok(self
            .offering()?
            .operations()
            .filter(|o| {
                o.operation == Operation::HistoricalPrices
                    && o.identity.plugin_id == "yfinance"
                    && o.identity.plugin_version == "0.3.0"
            })
            .map(|o| o.identity.clone())
            .collect())
    }
    pub async fn start_portfolio_history(
        &self,
        request: PortfolioHistoryRequest,
    ) -> Result<PortfolioHistoryResult> {
        let r = request.clone();
        if let Some(prior) = self
            .portfolio_effect(move |s| s.portfolio_history_request(&r))
            .await?
        {
            return self
                .portfolio_history_status(prior.key.portfolio_id, prior.id)
                .await;
        }
        let doc = self
            .portfolio_document(request.portfolio_id.clone())
            .await?;
        let providers = self.history_providers()?;
        let benchmark = match &doc.benchmark_instance_id {
            Some(id) => providers.iter().find(|p| &p.instance_id == id).cloned(),
            None if providers.len() == 1 => providers.first().cloned(),
            _ => None,
        };
        let key = HistoryKey::new(&doc, &request, benchmark)?;
        let anchor = chrono::Utc::now().with_timezone(&New_York).date_naive();
        if request.range.end > anchor {
            return Err(crate::portfolio::invalid(
                "historical endpoint cannot be in the future",
            ));
        }
        if request.refresh == HistoryRefreshMode::Missing
            && let Some(cached) = self
                .portfolio_history_latest(
                    request.portfolio_id.clone(),
                    request.account_id.clone(),
                    request.range.clone(),
                )
                .await?
            && cached.key == key
            && cached.created_at.with_timezone(&New_York).date_naive() == anchor
        {
            return Ok(cached);
        }
        let p = request.portfolio_id.clone();
        let r = request.clone();
        let k = key.clone();
        let (mut result, started, lease) = self
            .portfolio_effect(move |s| {
                let lease = s.portfolio_history_acquire(&p)?;
                let (result, started) = s.portfolio_history_begin(&r, &k, &lease)?;
                Ok((result, started, lease))
            })
            .await?;
        if !started {
            return Ok(result);
        }
        let (cancel, cancelled) = watch::channel(false);
        let (done_sender, done) = watch::channel(false);
        let admitted = {
            let mut a = lock(&self.inner.admission)?;
            if a.closed || a.histories.len() >= 4 {
                false
            } else {
                a.histories.insert(
                    result.id.clone(),
                    (
                        result.key.portfolio_id.clone(),
                        PortfolioJob { cancel, done },
                    ),
                );
                true
            }
        };
        if !admitted {
            result.status = HistoryStatus::Cancelled;
            return self
                .portfolio_effect(move |s| s.portfolio_history_finish(&result))
                .await;
        }
        let receipt = result.clone();
        let app = self.clone();
        tokio::spawn(async move {
            let _lease = lease;
            let stop = Stop {
                cancel: cancelled,
                deadline: lugus_agent::deadline::Deadline::after(
                    if app.limits().operation_timeout == std::time::Duration::MAX {
                        std::time::Duration::MAX
                    } else {
                        std::time::Duration::from_secs(300)
                    },
                ),
            };
            if let Err(e) = app
                .collect_history(&doc, &request, &mut result, anchor, &stop)
                .await
            {
                result.status = match e.kind {
                    ErrorKind::Cancelled => HistoryStatus::Cancelled,
                    ErrorKind::Timeout => HistoryStatus::TimedOut,
                    _ => HistoryStatus::Failed,
                };
                result.error = Some(e.message);
            }
            let id = result.id.clone();
            let mut failed = result.clone();
            if let Err(error) = app
                .portfolio_effect(move |s| s.portfolio_history_finish(&result))
                .await
            {
                failed.status = HistoryStatus::Failed;
                failed.error = Some(error.message);
                failed.evidence.clear();
                failed.issues.clear();
                failed.row_count = 0;
                failed.summary = Default::default();
                failed.baseline = None;
                failed.effective_end = None;
                let _ = app
                    .portfolio_effect(move |s| s.portfolio_history_finish(&failed))
                    .await;
            }
            done_sender.send_replace(true);
            if let Ok(mut admission) = lock(&app.inner.admission) {
                admission.histories.remove(&id);
            }
        });
        Ok(receipt)
    }
    async fn collect_history(
        &self,
        doc: &PortfolioDocument,
        request: &PortfolioHistoryRequest,
        result: &mut PortfolioHistoryResult,
        anchor: Day,
        stop: &Stop,
    ) -> Result<()> {
        let selected = doc
            .accounts
            .iter()
            .filter(|a| request.account_id.as_ref().is_none_or(|id| id == &a.id))
            .collect::<Vec<_>>();
        let funded_start = selected
            .iter()
            .filter_map(|a| match &a.ledger.opening {
                Opening::Existing { cash, lots } if cash.is_positive() || !lots.is_empty() => {
                    Some(a.ledger.start)
                }
                _ => a
                    .ledger
                    .events
                    .iter()
                    .filter(|e| matches!(e.kind, EventKind::Deposit { .. }))
                    .map(|e| e.date)
                    .min(),
            })
            .min()
            .unwrap_or(request.range.end);
        let start = request
            .range
            .start
            .unwrap_or(funded_start)
            .min(request.range.end);
        let baseline = start
            .pred_opt()
            .ok_or_else(|| crate::portfolio::invalid("history baseline outside supported range"))?;
        let query_start = selected
            .iter()
            .map(|a| a.ledger.start)
            .min()
            .unwrap_or(start)
            .min(baseline);
        if self.limits().operation_timeout != std::time::Duration::MAX
            && (anchor - query_start).num_days() >= 100_000
        {
            return Err(error(
                ErrorKind::ResourceLimit,
                "historical range exceeds limit",
            ));
        }
        let mut ids = BTreeSet::new();
        for account in &selected {
            if account.ledger.start > request.range.end {
                continue;
            }
            if let Opening::Existing { lots, .. } = &account.ledger.opening {
                for lot in lots {
                    ids.insert(lot.instrument_id.clone());
                }
            }
            for event in &account.ledger.events {
                if event.date <= request.range.end
                    && matches!(event.kind, EventKind::Buy { .. })
                    && let Some(id) = event.kind.instrument_id()
                {
                    ids.insert(id.to_owned());
                }
            }
        }
        if self.limits().operation_timeout != std::time::Duration::MAX && ids.len() > 100 {
            return Err(error(
                ErrorKind::ResourceLimit,
                "historical instrument limit exceeded",
            ));
        }
        let mut tasks = Vec::new();
        let providers = self.history_providers()?;
        for id in ids {
            let Some(binding) = doc
                .instruments
                .iter()
                .find(|i| i.id == id)
                .and_then(|i| i.binding.as_ref())
            else {
                continue;
            };
            if providers
                .iter()
                .any(|p| p.instance_id == binding.instance_id)
            {
                tasks.push((
                    Some(id),
                    binding.instance_id.clone(),
                    binding.native_id.clone(),
                ));
            }
        }
        if let Some(provider) = &result.key.benchmark_provider {
            tasks.push((
                None,
                provider.instance_id.clone(),
                lugus_financial::market_data::InstrumentId {
                    namespace: "yahoo:symbol".into(),
                    value: "^SP500TR".into(),
                },
            ));
        }
        let mut bundles = vec![];
        let mut rows = 0usize;
        let mut bytes = 0usize;
        let p = doc.header.id.clone();
        let cache = if request.refresh == HistoryRefreshMode::Missing {
            self.portfolio_effect(move |s| s.portfolio_history_cached(&p))
                .await?
        } else {
            None
        };
        let mut pending = Vec::new();
        for task in tasks {
            let reusable = cache
                .as_ref()
                .filter(|r| r.key.bindings_fingerprint == result.key.bindings_fingerprint)
                .and_then(|r| {
                    r.evidence
                        .iter()
                        .find(|e| e.instrument_id == task.0 && e.provider.instance_id == task.1)
                });
            let reused = if let Some(reference) = reusable {
                let scope =
                    self.scope(&format!("portfolio:{}", doc.header.id), &result.id, None)?;
                match self
                    .load_history_bundle(&scope, &reference.fetch_id, task.0.clone(), stop)
                    .await
                {
                    Ok(bundle)
                        if bundle.1.run.query.anchor == anchor
                            && bundle.1.run.query.start <= query_start =>
                    {
                        Some(bundle)
                    }
                    _ => None,
                }
            } else {
                None
            };
            if let Some(bundle) = reused {
                rows += bundle.1.items.len();
                bytes += serde_json::to_vec(&bundle.1)
                    .map_err(|_| crate::portfolio::invalid("history serialization failed"))?
                    .len();
                bundles.push(bundle);
            } else {
                pending.push(task);
            }
        }
        if self.limits().operation_timeout != std::time::Duration::MAX
            && (rows > 100_000 || bytes > 128 * 1024 * 1024)
        {
            return Err(error(
                ErrorKind::ResourceLimit,
                "cached history exceeds budget",
            ));
        }
        // Two in-flight source jobs at most, still subject to the managed worker's global budget.
        for chunk in pending.chunks(2) {
            stop.check()?;
            let first = self.fetch_history_bundle(
                result,
                &chunk[0],
                query_start,
                request.range.end,
                anchor,
                stop,
            );
            let responses = if let Some(second) = chunk.get(1) {
                let (a, b) = tokio::join!(
                    first,
                    self.fetch_history_bundle(
                        result,
                        second,
                        query_start,
                        request.range.end,
                        anchor,
                        stop
                    )
                );
                vec![a, b]
            } else {
                vec![first.await]
            };
            for response in responses {
                match response {
                    Ok(bundle) => {
                        rows += bundle.1.items.len();
                        bytes += serde_json::to_vec(&bundle.1)
                            .map_err(|_| crate::portfolio::invalid("history serialization failed"))?
                            .len();
                        if self.limits().operation_timeout != std::time::Duration::MAX
                            && (rows > 100_000 || bytes > 128 * 1024 * 1024)
                        {
                            return Err(error(
                                ErrorKind::ResourceLimit,
                                "portfolio history evidence exceeds budget",
                            ));
                        }
                        bundles.push(bundle);
                    }
                    Err(e)
                        if matches!(
                            e.kind,
                            ErrorKind::Cancelled | ErrorKind::Timeout | ErrorKind::ResourceLimit
                        ) =>
                    {
                        return Err(e);
                    }
                    Err(e) => {
                        result.error = Some(e.message);
                    }
                }
            }
        }
        stop.check()?;
        let end = bundles
            .iter()
            .filter_map(|(_, p)| p.run.manifest.as_ref().map(|m| m.last_completed_session))
            .min()
            .unwrap_or(request.range.end)
            .min(request.range.end);
        if end < baseline {
            return Err(crate::portfolio::invalid(
                "no completed session in requested history range",
            ));
        }
        result.evidence = bundles.iter().map(|(r, _)| r.clone()).collect();
        result.input_fingerprint = Some(fingerprint(&(&result.key, &result.evidence))?);
        let doc = doc.clone();
        let account = request.account_id.clone();
        let cancel = stop.clone();
        let computed = tokio::task::spawn_blocking(move || {
            let owned = history_inputs(&doc, account.as_deref(), baseline, end, &bundles, &|| {
                cancel.stopped()
            })?;
            let days = historical_values_cancellable(
                &ValuationHistoryInput {
                    ledgers: &owned.ledgers,
                    closes: &owned.closes,
                    benchmark: &owned.benchmark,
                    baseline: owned.baseline,
                    end: owned.end,
                },
                &|| cancel.stopped(),
            )
            .map_err(crate::portfolio::engine)?;
            calculate_performance_cancellable(&days, &|| cancel.stopped())
                .map_err(crate::portfolio::engine)
        })
        .await
        .map_err(|_| error(ErrorKind::Storage, "historical calculation task failed"))?;
        stop.check()?;
        let series = computed?;
        result.baseline = Some(series.baseline);
        result.effective_end = Some(series.end);
        result.summary = series.summary;
        result.row_count = series.points.len();
        let mut seen = BTreeSet::new();
        for point in &series.points {
            for issue in &point.issues {
                result.issue_count += 1;
                let key = format!(
                    "{:?}:{:?}:{:?}",
                    issue.code, issue.instrument_id, issue.account_id
                );
                if result.issues.len() < 20 && seen.insert(key) {
                    result.issues.push(issue.clone());
                }
            }
        }
        if result.key.benchmark_provider.is_none() && providers.len() > 1 {
            result.issue_count += 1;
            if result.issues.len() < 20 {
                result.issues.push(HistoryIssue {
                    code: HistoryIssueCode::BenchmarkSelectionRequired,
                    date: end,
                    instrument_id: None,
                    account_id: None,
                });
            }
        }
        let mut offset = 0;
        while offset < series.points.len() {
            stop.check()?;
            let mut count = (series.points.len() - offset)
                .min(200)
                .min(self.inner.limits.max_read_page_items);
            while crate::agent_contract::check_serialized_size(
                &&series.points[offset..offset + count],
                self.inner.limits.max_read_page_bytes,
            )
            .is_err()
            {
                if count <= 1 {
                    return Err(error(
                        ErrorKind::ResourceLimit,
                        "historical row exceeds storage budget",
                    ));
                }
                count = (count / 2).max(1);
            }
            let id = result.id.clone();
            let chunk = series.points[offset..offset + count].to_vec();
            self.portfolio_effect(move |s| s.portfolio_history_save_rows(&id, offset, &chunk))
                .await?;
            offset += count;
        }
        stop.check()?;
        result.status = if result.error.is_some()
            && result.summary.portfolio_return_percent.is_none()
            && result.summary.benchmark_return_percent.is_none()
        {
            HistoryStatus::Failed
        } else if result.summary.portfolio_return_percent.is_some()
            && result.summary.benchmark_return_percent.is_some()
            && series
                .points
                .iter()
                .flat_map(|p| &p.issues)
                .all(|i| i.code == HistoryIssueCode::ZeroCapital && i.date == baseline)
            && result.error.is_none()
        {
            HistoryStatus::Complete
        } else {
            HistoryStatus::Partial
        };
        Ok(())
    }
    async fn fetch_history_bundle(
        &self,
        result: &PortfolioHistoryResult,
        task: &(
            Option<String>,
            String,
            lugus_financial::market_data::InstrumentId,
        ),
        start: Day,
        end: Day,
        anchor: Day,
        stop: &Stop,
    ) -> Result<(HistoryEvidenceRef, HistoryReadPage)> {
        let scope = self.scope(
            &format!("portfolio:{}", result.key.portfolio_id),
            &format!("{}:{}", result.id, task.0.as_deref().unwrap_or("benchmark")),
            None,
        )?;
        let job = self.submit_manual(
            &scope,
            FetchCommand::HistoricalPrices {
                instance_id: task.1.clone(),
                query: HistoryQuery {
                    instrument: task.2.clone(),
                    start,
                    end,
                    anchor,
                    cursor: None,
                    page_size: 200,
                },
            },
        )?;
        let _guard = ChildFetch {
            app: self.clone(),
            scope: scope.clone(),
            id: job.id.clone(),
        };
        let status = stop.wait(self.wait(&scope, &job.id)).await?;
        let id = status
            .fetch_id
            .ok_or_else(|| crate::portfolio::invalid("historical fetch did not save receipt"))?;
        self.load_history_bundle(&scope, &id, task.0.clone(), stop)
            .await
    }
    async fn load_history_bundle(
        &self,
        scope: &Scope,
        id: &str,
        instrument_id: Option<String>,
        stop: &Stop,
    ) -> Result<(HistoryEvidenceRef, HistoryReadPage)> {
        let fetch = self.read_fetch(scope, id).await?;
        if let Some(e) = fetch.error {
            return Err(e);
        }
        let mut page = self
            .history_evidence_page(
                scope,
                id,
                PageRequest {
                    offset: 0,
                    limit: 200,
                },
            )
            .await?;
        while let Some(offset) = page.next_offset {
            stop.check()?;
            let next = self
                .history_evidence_page(scope, id, PageRequest { offset, limit: 200 })
                .await?;
            if page.items.len() != offset
                || (self.limits().operation_timeout != std::time::Duration::MAX
                    && page.items.len() + next.items.len() > 100_000)
            {
                return Err(error(
                    ErrorKind::ResourceLimit,
                    "historical evidence pagination exceeds limit",
                ));
            }
            page.next_offset = next.next_offset;
            page.items.extend(next.items);
        }
        let manifest = page
            .run
            .manifest
            .as_ref()
            .ok_or_else(|| crate::portfolio::invalid("history manifest missing"))?;
        Ok((
            HistoryEvidenceRef {
                manifest: Some(manifest.clone()),
                source_url: page.items.first().map(|r| r.day.source_url.clone()),
                instrument_id,
                fetch_id: id.to_owned(),
                run_id: page.run.id.to_string(),
                provider: fetch.provider,
                manifest_fingerprint: fingerprint(manifest)?,
            },
            page,
        ))
    }
    pub async fn portfolio_history_status(
        &self,
        p: String,
        id: String,
    ) -> Result<PortfolioHistoryResult> {
        let saved = id.clone();
        let mut result = self
            .portfolio_effect(move |s| s.portfolio_history_read(&p, &saved))
            .await?;
        if result.status == HistoryStatus::Running
            && !lock(&self.inner.admission)?.histories.contains_key(&id)
        {
            result.status = HistoryStatus::InterruptedOrExternal;
        }
        Ok(result)
    }
    pub fn cancel_portfolio_history(&self, p: String, id: String) -> Result<()> {
        crate::portfolio::id(&p)?;
        crate::portfolio::id(&id)?;
        if let Some((portfolio, job)) = lock(&self.inner.admission)?.histories.get(&id) {
            if portfolio != &p {
                return Err(error(
                    ErrorKind::ScopeMismatch,
                    "history job does not belong to portfolio",
                ));
            }
            job.cancel.send_replace(true);
        }
        Ok(())
    }
    pub async fn portfolio_history_latest(
        &self,
        p: String,
        a: Option<String>,
        range: HistoryRange,
    ) -> Result<Option<PortfolioHistoryResult>> {
        self.portfolio_effect(move |s| s.portfolio_history_latest(&p, a.as_deref(), &range))
            .await
    }
    pub async fn portfolio_history_page(
        &self,
        p: String,
        id: String,
        page: PageRequest,
    ) -> Result<PortfolioHistoryPage> {
        self.portfolio_effect(move |s| s.portfolio_history_page(&p, &id, page))
            .await
    }
    pub async fn portfolio_history_evidence(
        &self,
        p: String,
        id: String,
        page: PageRequest,
    ) -> Result<PortfolioPage<HistoryEvidenceRef>> {
        self.portfolio_effect(move |s| s.portfolio_history_evidence(&p, &id, page))
            .await
    }
}
