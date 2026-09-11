//! Effectful application orchestration; the model never supplies provider commands.
use super::{
    ResearchIntent, SubjectMention, Workflow, date_range, resolution_input, resolve_candidate,
};
use crate::{conversations::RunRecord, *};
use chrono::{DateTime, Utc};
use lugus_financial::{
    instruments::InstrumentLookup,
    market_data::InstrumentId,
    resolution::catalog::CatalogEntry,
    selection::{MetricQuery, PeriodSelection, PriceSeries},
};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedSubject {
    pub mention: SubjectMention,
    pub company: CatalogEntry,
    pub resolution_dataset_id: String,
    pub binding: Option<BindingRecord>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedDataset {
    pub purpose: String,
    pub page: DatasetPage,
    pub complete_in_context: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedResearch {
    pub policy: String,
    pub run_id: String,
    pub prepared_at: DateTime<Utc>,
    pub intent: ResearchIntent,
    pub subjects: Vec<PreparedSubject>,
    pub datasets: Vec<PreparedDataset>,
    #[serde(default)]
    pub views: Vec<ViewReceipt>,
    pub fetches: Vec<FetchReference>,
    pub issues: Vec<AppError>,
    pub clarification: Option<String>,
}

struct Preparation<'a> {
    app: &'a Application,
    run: &'a RunRecord,
    offering: Offering,
    cancel: watch::Receiver<bool>,
    next: usize,
    previous: Option<&'a PreparedResearch>,
    output: PreparedResearch,
}
fn failure(kind: ErrorKind, message: &str) -> AppError {
    AppError::new(kind, message, false)
}

/// All financial reads are prepared before the analysis runtime is created.
pub async fn prepare_research(
    app: &Application,
    run: &RunRecord,
    intent: ResearchIntent,
    previous: Option<&PreparedResearch>,
    cancel: watch::Receiver<bool>,
) -> Result<PreparedResearch> {
    intent.validate(run.created_at.date_naive())?;
    let mut work = Preparation {
        app,
        run,
        offering: app.offering()?,
        cancel,
        next: 0,
        previous,
        output: PreparedResearch {
            policy: "application-research-v1".into(),
            run_id: run.id.clone(),
            prepared_at: run.created_at,
            clarification: intent.clarification.clone(),
            intent: intent.clone(),
            subjects: vec![],
            datasets: vec![],
            views: vec![],
            fetches: vec![],
            issues: vec![],
        },
    };
    if matches!(intent.workflow, Workflow::Conversation | Workflow::Clarify) {
        if let Some(previous) = previous {
            // Conversational interludes must not erase verified company context.
            work.output.subjects = previous.subjects.clone();
            work.output.datasets = previous.datasets.clone();
            work.output.views = previous.views.clone();
            work.output.fetches = previous.fetches.clone();
            work.output.issues = previous.issues.clone();
        }
        return Ok(work.output);
    }
    // Resolve every subject before any company-specific facts or prices are fetched.
    for mention in &intent.subjects {
        work.check_cancel()?;
        match work.resolve(mention).await {
            Ok(subject) => work.output.subjects.push(subject),
            Err(error) => {
                if error.kind == ErrorKind::Cancelled {
                    return Err(error);
                }
                work.output.clarification = Some(error.message.clone());
                work.output.issues.push(error);
            }
        }
    }
    if work.output.clarification.is_some() {
        return Ok(work.output);
    }
    if work.output.subjects.len() == 2
        && work.output.subjects[0].company.candidate.identifier
            == work.output.subjects[1].company.candidate.identifier
    {
        work.output.clarification = Some(
            "Both mentions resolve to the same company. Which second company should be compared?"
                .into(),
        );
        return Ok(work.output);
    }
    let (start, end) = date_range(&intent, run.created_at.date_naive())?;
    for mut subject in work.output.subjects.clone() {
        work.check_cancel()?;
        if intent.workflow != Workflow::Prices
            && let Err(error) = work.fundamentals(&subject, start, end).await
        {
            if error.kind == ErrorKind::Cancelled {
                return Err(error);
            }
            work.output.issues.push(error);
        }
        if let Err(error) = work.prices(&mut subject, start, end).await {
            if error.kind == ErrorKind::Cancelled {
                return Err(error);
            }
            work.output.issues.push(error);
        }
        if let Some(target) = work
            .output
            .subjects
            .iter_mut()
            .find(|s| s.company.company == subject.company.company)
        {
            *target = subject;
        }
    }
    work.check_cancel()?;
    Ok(work.output)
}

impl Preparation<'_> {
    fn check_cancel(&self) -> Result<()> {
        if *self.cancel.borrow() {
            Err(failure(
                ErrorKind::Cancelled,
                "Research preparation cancelled",
            ))
        } else {
            Ok(())
        }
    }
    fn scope(&mut self) -> Scope {
        self.next += 1;
        Scope {
            workspace_id: self.run.workspace_id.clone(),
            request_id: format!("prepare:{}:{}", self.run.id, self.next),
            run_id: Some(self.run.id.clone()),
        }
    }
    fn provider(&self, operation: Operation) -> Result<String> {
        let mut eligible = self
            .offering
            .operations()
            .filter(|p| p.operation == operation);
        let provider = eligible.next().ok_or_else(|| {
            failure(
                ErrorKind::Unavailable,
                "No active provider supports this research data",
            )
        })?;
        if eligible.next().is_some() {
            return Err(failure(
                ErrorKind::AmbiguousProvider,
                "Multiple providers support this data; configure a single source for this capability",
            ));
        }
        Ok(provider.identity.instance_id.clone())
    }
    async fn fetch(&mut self, command: FetchCommand) -> Result<FetchReference> {
        self.check_cancel()?;
        let scope = self.scope();
        // A short freshness window reuses exact successful requests; no query/provider substitution.
        if let Some(previous) = self.previous {
            for old in &previous.fetches {
                let age = self.run.created_at.signed_duration_since(old.created_at);
                if old.error.is_none()
                    && age.num_seconds() >= 0
                    && age.num_seconds() < 900
                    && serde_json::to_value(&old.command).ok()
                        == serde_json::to_value(&command).ok()
                    && self
                        .offering
                        .operations()
                        .any(|o| o.operation == command.operation() && o.identity == old.provider)
                {
                    let cached = self.app.read_fetch(&scope, &old.id).await?;
                    self.output.fetches.push(cached.clone());
                    return Ok(cached);
                }
            }
        }
        let job = self.app.submit(&scope, &self.offering, command)?;
        let mut guard = CancelFetch {
            app: self.app.clone(),
            scope: scope.clone(),
            id: Some(job.id.clone()),
        };
        let status = tokio::select! {
            biased;
            _ = cancelled(&mut self.cancel) => { return Err(failure(ErrorKind::Cancelled, "Research preparation cancelled")); }
            result = self.app.wait(&scope, &job.id) => result?,
        };
        guard.id = None;
        let id = status.fetch_id.ok_or_else(|| {
            status.error.clone().unwrap_or_else(|| {
                failure(ErrorKind::MissingData, "Fetch produced no durable receipt")
            })
        })?;
        let fetch = self.app.read_fetch(&scope, &id).await?;
        self.output.fetches.push(fetch.clone());
        Ok(fetch)
    }
    async fn dataset(
        &mut self,
        fetch: &FetchReference,
        projection: DatasetProjection,
        purpose: &str,
        view: Option<ViewKind>,
    ) -> Result<DatasetPage> {
        self.check_cancel()?;
        let scope = self.scope();
        let header = self
            .app
            .create_dataset(&scope, &fetch.id, projection)
            .await?;
        // Small initial slices; analysis may page through the already-frozen dataset offline.
        let page = self
            .app
            .read_dataset(
                &scope,
                &header.id,
                PageRequest {
                    offset: 0,
                    limit: 4.min(self.app.limits().max_read_page_items),
                },
            )
            .await?;
        self.output.datasets.push(PreparedDataset {
            purpose: purpose.into(),
            complete_in_context: page.next_offset.is_none(),
            page: page.clone(),
        });
        if let Some(kind) = view {
            self.check_cancel()?;
            let view_scope = self.scope();
            match self
                .app
                .open_view(
                    &view_scope,
                    OpenViewRequest {
                        dataset_id: header.id,
                        kind,
                    },
                )
                .await
            {
                Ok(view) => self.output.views.push(view),
                Err(error) => self.output.issues.push(error),
            }
        }
        Ok(page)
    }
    async fn resolve(&mut self, mention: &SubjectMention) -> Result<PreparedSubject> {
        let instance_id = self.provider(Operation::Resolve)?;
        let fetch = self
            .fetch(FetchCommand::Resolve {
                instance_id,
                input: resolution_input(&mention.text)?,
            })
            .await?;
        if let Some(error) = &fetch.error {
            return Err(error.clone());
        }
        let run_id = run_id(&fetch, RunKind::Resolution)?;
        let page = self
            .dataset(
                &fetch,
                DatasetProjection::Resolution { run_id },
                "company_identity",
                None,
            )
            .await?;
        let company = resolve_candidate(
            page.header.resolution_status.as_deref(),
            &page.rows,
            page.next_offset,
        )
        .map_err(|mut error| {
            let suggestions: Vec<_> = page
                .rows
                .iter()
                .filter_map(|r| match r {
                    DatasetRow::Candidate { entry } => Some(format!(
                        "{} ({})",
                        entry.candidate.name,
                        entry
                            .candidate
                            .listings
                            .iter()
                            .map(|l| l.ticker.value.clone())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                    _ => None,
                })
                .collect();
            if !suggestions.is_empty() {
                error = failure(
                    ErrorKind::NeedsAttention,
                    &format!(
                        "Please confirm the exact company/ticker. Source candidates: {}",
                        suggestions.join("; ")
                    ),
                );
            }
            error
        })?;
        if subject_exchange_missing(mention, &company) {
            return Err(failure(
                ErrorKind::NeedsAttention,
                "The requested exchange does not match a sourced listing. Please confirm the company ticker and exchange.",
            ));
        }
        Ok(PreparedSubject {
            mention: mention.clone(),
            company,
            resolution_dataset_id: page.header.id,
            binding: None,
        })
    }
    async fn fundamentals(
        &mut self,
        subject: &PreparedSubject,
        start: chrono::NaiveDate,
        end: chrono::NaiveDate,
    ) -> Result<()> {
        let query = Query {
            company: subject.company.candidate.identifier.clone(),
            filed_from: start,
            filed_to: end,
            forms: vec![],
            page_size: 100,
            cursor: None,
        };
        // Filings and structured facts fail independently, so one source capability can still help.
        let filings = async {
            let instance_id = self.provider(Operation::Filings)?;
            let fetch = self
                .fetch(FetchCommand::Filings {
                    instance_id,
                    query: query.clone(),
                })
                .await?;
            if let Some(error) = &fetch.error {
                self.output.issues.push(error.clone());
            }
            let run_id = run_id(&fetch, RunKind::Financial)?;
            self.dataset(
                &fetch,
                DatasetProjection::Filings { run_id },
                "filings",
                None,
            )
            .await?;
            Ok::<(), AppError>(())
        }
        .await;
        if let Err(error) = filings {
            if error.kind == ErrorKind::Cancelled {
                return Err(error);
            }
            self.output.issues.push(error);
        }
        let instance_id = self.provider(Operation::Facts)?;
        let fetch = self
            .fetch(FetchCommand::Facts {
                instance_id,
                query: query.clone(),
            })
            .await?;
        if let Some(error) = &fetch.error {
            self.output.issues.push(error.clone());
        }
        let run_id = run_id(&fetch, RunKind::Financial)?;
        if let Err(error) = self
            .dataset(
                &fetch,
                DatasetProjection::AllFacts { run_id },
                "all_reported_facts",
                Some(ViewKind::DataTable),
            )
            .await
        {
            if error.kind == ErrorKind::Cancelled {
                return Err(error);
            }
            self.output.issues.push(error);
        }
        for (concept, periods) in [
            ("Assets", PeriodSelection::Instants),
            ("Liabilities", PeriodSelection::Instants),
            (
                "CashAndCashEquivalentsAtCarryingValue",
                PeriodSelection::Instants,
            ),
            ("LongTermDebtCurrent", PeriodSelection::Instants),
            ("LongTermDebtNoncurrent", PeriodSelection::Instants),
            ("ShortTermBorrowings", PeriodSelection::Instants),
            (
                "RevenueFromContractWithCustomerExcludingAssessedTax",
                PeriodSelection::Durations,
            ),
            ("Revenues", PeriodSelection::Durations),
            ("NetIncomeLoss", PeriodSelection::Durations),
        ] {
            let metric = MetricQuery {
                scope: query.clone(),
                namespace: "us-gaap".into(),
                concept: concept.into(),
                unit: "USD".into(),
                periods,
            };
            self.dataset(
                &fetch,
                DatasetProjection::Facts {
                    run_id,
                    query: metric,
                },
                concept,
                None,
            )
            .await?;
        }
        Ok(())
    }
    async fn prices(
        &mut self,
        subject: &mut PreparedSubject,
        start: chrono::NaiveDate,
        end: chrono::NaiveDate,
    ) -> Result<()> {
        let listings: Vec<_> = subject
            .company
            .candidate
            .listings
            .iter()
            .filter(|listing| {
                let exchange = subject.mention.exchange.as_deref();
                exchange.is_none_or(|e| {
                    listing
                        .exchange
                        .as_ref()
                        .is_some_and(|x| x.value.eq_ignore_ascii_case(e))
                })
            })
            .filter(|listing| {
                let text = subject.mention.text.trim_start_matches('$');
                // An explicit ticker chooses its listing; names/CIKs need a single source listing.
                !subject
                    .company
                    .candidate
                    .listings
                    .iter()
                    .any(|l| l.ticker.value.eq_ignore_ascii_case(text))
                    || listing.ticker.value.eq_ignore_ascii_case(text)
            })
            .collect();
        let [listing] = listings.as_slice() else {
            return Err(failure(
                ErrorKind::NeedsAttention,
                "Choose a single listing ticker and exchange before requesting prices",
            ));
        };
        let instrument_provider = self.provider(Operation::InstrumentLookup)?;
        let price_provider = self.provider(Operation::Prices)?;
        // Initial native-identifier adapter is explicit; other providers can add adapters without changing orchestration.
        let identity = self
            .offering
            .operations()
            .find(|p| p.operation == Operation::Prices && p.identity.instance_id == price_provider)
            .unwrap();
        if identity.identity.plugin_id != "yfinance" || instrument_provider != price_provider {
            return Err(failure(
                ErrorKind::Unsupported,
                "Price preparation needs a configured market provider with a supported native-identifier adapter and instrument lookup",
            ));
        }
        let cached = self.previous.and_then(|previous| {
            let old_subject = previous.subjects.iter().find(|old| old.company.candidate.identifier == subject.company.candidate.identifier)?;
            let binding = old_subject.binding.as_ref()?;
            if &binding.listing != *listing || binding.instrument.provider != identity.identity { return None; }
            let fetch = previous.fetches.iter().find(|fetch| {
                let age = self.run.created_at.signed_duration_since(fetch.created_at).num_seconds();
                fetch.error.is_none() && (0..900).contains(&age) && fetch.binding_id.as_deref() == Some(&binding.id)
                    && matches!(&fetch.command, FetchCommand::Prices { query, .. } if query.start == start && query.end == end && query.instrument == binding.instrument.request.instrument)
            })?;
            Some((binding.clone(), fetch.clone()))
        });
        if let Some((binding, fetch)) = cached {
            let scope = self.scope();
            let status = self.app.read_binding(&scope, &binding.id).await?;
            if status.status == BindingStatus::Active {
                let fetch = self.app.read_fetch(&scope, &fetch.id).await?;
                let FetchCommand::Prices { query, .. } = &fetch.command else {
                    unreachable!()
                };
                self.output.fetches.push(fetch.clone());
                subject.binding = Some(binding);
                self.dataset(
                    &fetch,
                    DatasetProjection::Prices {
                        run_id: run_id(&fetch, RunKind::Market)?,
                        query: query.clone(),
                        series: PriceSeries::Close,
                    },
                    "daily_close",
                    Some(ViewKind::PriceChart),
                )
                .await?;
                return Ok(());
            }
        }
        let instrument = InstrumentId {
            namespace: "yahoo:symbol".into(),
            value: listing.ticker.value.clone(),
        };
        let metadata = self
            .fetch(FetchCommand::InstrumentLookup {
                instance_id: instrument_provider,
                query: InstrumentLookup { instrument },
            })
            .await?;
        if let Some(error) = metadata.error {
            return Err(error);
        }
        self.check_cancel()?;
        let binding_scope = self.scope();
        let binding = self
            .app
            .create_binding(
                &binding_scope,
                &BindRequest {
                    company_dataset_id: subject.resolution_dataset_id.clone(),
                    company_observation_id: subject.company.observation_id,
                    listing: (*listing).clone(),
                    instrument_fetch_id: metadata.id,
                    supersedes: None,
                },
            )
            .await?;
        subject.binding = Some(binding.clone());
        let scope = self.scope();
        let job = self
            .app
            .fetch_bound_prices(
                &scope,
                &self.offering,
                &crate::BoundPriceRequest {
                    binding_id: binding.id,
                    start,
                    end,
                    page_size: 100,
                },
            )
            .await?;
        let mut guard = CancelFetch {
            app: self.app.clone(),
            scope: scope.clone(),
            id: Some(job.id.clone()),
        };
        let status = tokio::select! {
            biased;
            _ = cancelled(&mut self.cancel) => { return Err(failure(ErrorKind::Cancelled, "Research preparation cancelled")); }
            result = self.app.wait(&scope, &job.id) => result?,
        };
        guard.id = None;
        let id = status
            .fetch_id
            .ok_or_else(|| failure(ErrorKind::MissingData, "Price fetch produced no receipt"))?;
        let fetch = self.app.read_fetch(&scope, &id).await?;
        self.output.fetches.push(fetch.clone());
        if let Some(error) = &fetch.error {
            self.output.issues.push(error.clone());
        }
        let run_id = run_id(&fetch, RunKind::Market)?;
        let FetchCommand::Prices { query, .. } = &fetch.command else {
            return Err(failure(
                ErrorKind::InvalidInput,
                "Invalid stored price fetch",
            ));
        };
        self.dataset(
            &fetch,
            DatasetProjection::Prices {
                run_id,
                query: query.clone(),
                series: PriceSeries::Close,
            },
            "daily_close",
            Some(ViewKind::PriceChart),
        )
        .await?;
        Ok(())
    }
}
fn subject_exchange_missing(mention: &SubjectMention, company: &CatalogEntry) -> bool {
    mention.exchange.as_ref().is_some_and(|exchange| {
        !company.candidate.listings.iter().any(|listing| {
            listing
                .exchange
                .as_ref()
                .is_some_and(|e| e.value.eq_ignore_ascii_case(exchange))
        })
    })
}
fn run_id(fetch: &FetchReference, kind: RunKind) -> Result<i64> {
    // Resolution may contain a completed empty ticker search followed by name fallback.
    fetch
        .runs
        .iter()
        .rev()
        .find(|r| r.kind == kind)
        .map(|r| r.id)
        .ok_or_else(|| {
            failure(
                ErrorKind::MissingData,
                "No saved ingestion run exists for this evidence",
            )
        })
}
struct CancelFetch {
    app: Application,
    scope: Scope,
    id: Option<String>,
}
impl Drop for CancelFetch {
    fn drop(&mut self) {
        if let Some(id) = &self.id {
            let _ = self.app.cancel(&self.scope, id);
        }
    }
}
async fn cancelled(receiver: &mut watch::Receiver<bool>) {
    while !*receiver.borrow_and_update() {
        if receiver.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
