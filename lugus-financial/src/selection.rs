//! Pure, versioned current-stored selection. No public-as-of or provider reconciliation.
//!
//! Read exact run membership through [`SelectionRepository`], then call [`select_daily`]
//! or [`select_facts`]. `pinned` identifies an explicit compatible historical run,
//! including incomplete evidence; `None` chooses the latest initiated completed run.
//! Serialize the owned [`SelectionManifest`] to freeze policy and evidence references.
//! A refresh creates a separate value and never reselects an existing manifest.
//!
//! Run completion and source coverage are independent. A completed source response
//! with partial coverage remains eligible, preserving its explicit coverage marker.
//! Transport cursors and page sizes are normalized out of semantic selection queries.
use crate::{
    domain::*,
    error::{Error, ErrorKind, Result},
    market_data::*,
    storage::{ObservationRetrieval, Run, RunStatus, market::MarketRun},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunContext {
    pub repository_id: String,
    pub sequence: i64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence<T> {
    pub retrieval: ObservationRetrieval,
    pub value: T,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialRunEvidence {
    pub context: RunContext,
    pub run: Run,
    pub facts: Vec<Evidence<Fact>>,
    pub filings: Vec<Evidence<Filing>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketRunEvidence {
    pub context: RunContext,
    pub run: MarketRun,
    pub prices: Vec<Evidence<PriceBar>>,
}
/// Repository adapters return exact run membership, never a union snapshot.
pub trait SelectionRepository {
    fn financial_runs(&self, provider: &ProviderIdentity) -> Result<Vec<FinancialRunEvidence>>;
    fn market_runs(&self, provider: &ProviderIdentity) -> Result<Vec<MarketRunEvidence>>;
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReference {
    pub context: RunContext,
    pub kind: String,
    pub run_id: i64,
    pub status: RunStatus,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceReference {
    pub observation_id: i64,
    pub kind: String,
    pub fingerprint: String,
    pub source_url: String,
    pub retrieved_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionManifest {
    pub policy: String,
    pub provider: ProviderIdentity,
    pub query: serde_json::Value,
    pub selected_run: Option<RunReference>,
    pub available_runs: Vec<RunReference>,
    pub observations: Vec<EvidenceReference>,
    /// Selected market run source coverage; independent of ingestion completion.
    /// None for fundamentals, absent datasets, or runs without source coverage.
    pub source_coverage: Option<PriceCoverage>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceSeries {
    Close,
    AdjustedClose,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailySelection {
    pub manifest: SelectionManifest,
    pub series: PriceSeries,
    pub prices: Vec<Evidence<PriceBar>>,
    pub values: Vec<Option<Decimal>>,
    pub conflicts: Vec<String>,
    pub coverage: Option<PriceCoverage>,
}
impl DailySelection {
    /// Latest source-reported close, independent of the requested plotted series.
    /// Adjusted-close values are available separately in `values`, including gaps.
    /// None for empty/conflicting datasets. This is a dated close, never a live quote.
    pub fn latest_close(&self) -> Option<&Evidence<PriceBar>> {
        if self.conflicts.is_empty() {
            self.prices.last()
        } else {
            None
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricQuery {
    pub scope: Query,
    pub namespace: String,
    pub concept: String,
    pub unit: String,
    pub periods: PeriodSelection,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PeriodSelection {
    LatestInstant,
    Instants,
    Durations,
    Exact { period: Period },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactGroup {
    pub period: Period,
    pub filed: NaiveDate,
    pub value: Option<Decimal>,
    pub candidates: Vec<Evidence<Fact>>,
    pub conflict: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactSelection {
    pub manifest: SelectionManifest,
    pub groups: Vec<FactGroup>,
}
/// Canonical numeric equality for arbitrary-size validated decimals, including negative zero.
pub fn decimal_key(value: &Decimal) -> String {
    let raw = value.as_str();
    let negative = raw.starts_with('-');
    let raw = raw.trim_start_matches('-');
    let (integer, fraction) = raw.split_once('.').unwrap_or((raw, ""));
    let integer = integer.trim_start_matches('0');
    let fraction = fraction.trim_end_matches('0');
    format!(
        "{}{}{}{}",
        if negative && (!integer.is_empty() || !fraction.is_empty()) {
            "-"
        } else {
            ""
        },
        if integer.is_empty() { "0" } else { integer },
        if fraction.is_empty() { "" } else { "." },
        fraction
    )
}
pub fn financial_contains(run: &Query, q: &Query) -> bool {
    run.company == q.company
        && run.filed_from <= q.filed_from
        && run.filed_to >= q.filed_to
        && (run.forms.is_empty()
            || (!q.forms.is_empty() && q.forms.iter().all(|f| run.forms.contains(f))))
}
pub fn price_contains(run: &PriceQuery, q: &PriceQuery) -> bool {
    run.instrument == q.instrument && run.start <= q.start && run.end >= q.end
}
fn reference<T>(e: &Evidence<T>, url: &str) -> EvidenceReference {
    EvidenceReference {
        observation_id: e.retrieval.observation_id,
        kind: e.retrieval.kind.clone(),
        fingerprint: e.retrieval.fingerprint.clone(),
        source_url: url.into(),
        retrieved_at: e.retrieval.retrieved_at,
    }
}
fn run_ref(
    context: &RunContext,
    kind: &str,
    id: i64,
    status: RunStatus,
    error: &Option<String>,
) -> RunReference {
    RunReference {
        context: context.clone(),
        kind: kind.into(),
        run_id: id,
        status,
        error: error.clone(),
    }
}
fn choose(runs: &[RunReference], pinned: Option<i64>) -> Result<Option<RunReference>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut repository = None;
    for r in runs {
        if repository.is_some_and(|id| id != &r.context.repository_id)
            || !seen.insert(r.context.sequence)
        {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "mixed repository identities or duplicate ingestion sequences",
            ));
        }
        repository = Some(&r.context.repository_id);
    }
    if let Some(id) = pinned {
        return runs
            .iter()
            .find(|r| r.run_id == id)
            .cloned()
            .map(Some)
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::NotFound,
                    "pinned run does not contain the requested scope",
                )
            });
    }
    Ok(runs
        .iter()
        .filter(|r| r.status == RunStatus::Complete)
        .max_by_key(|r| r.context.sequence)
        .cloned())
}
fn manifest(
    policy: &str,
    provider: &ProviderIdentity,
    query: serde_json::Value,
    mut runs: Vec<RunReference>,
    pinned: Option<i64>,
) -> Result<SelectionManifest> {
    runs.sort_by_key(|r| r.context.sequence);
    let selected_run = choose(&runs, pinned)?;
    let mut limitations = vec![
        "Current stored evidence; no historical public-information guarantee.".into(),
        "Run completion does not guarantee source coverage.".into(),
    ];
    if selected_run
        .as_ref()
        .is_some_and(|r| r.status != RunStatus::Complete)
    {
        limitations.push("Explicit historical run is incomplete; partial evidence only.".into());
    }
    Ok(SelectionManifest {
        policy: policy.into(),
        provider: provider.clone(),
        query,
        selected_run,
        available_runs: runs,
        observations: vec![],
        source_coverage: None,
        limitations,
    })
}
fn validate_provider(p: &ProviderIdentity) -> Result<()> {
    if [&p.instance_id, &p.plugin_id, &p.plugin_version]
        .iter()
        .any(|v| v.trim().is_empty())
    {
        Err(Error::new(
            ErrorKind::InvalidRequest,
            "full provider identity required",
        ))
    } else {
        Ok(())
    }
}
pub fn select_daily(
    provider: &ProviderIdentity,
    query: &PriceQuery,
    series: PriceSeries,
    runs: &[MarketRunEvidence],
    pinned: Option<i64>,
) -> Result<DailySelection> {
    validate_provider(provider)?;
    query.validate()?;
    let eligible: Vec<_> = runs
        .iter()
        .filter(|r| r.run.provider == *provider && price_contains(&r.run.query, query))
        .collect();
    let mut q = query.clone();
    q.cursor = None;
    q.page_size = 1;
    let refs = eligible
        .iter()
        .map(|r| run_ref(&r.context, "market", r.run.id, r.run.status, &r.run.error))
        .collect();
    let mut manifest = manifest(
        "daily-series:1",
        provider,
        serde_json::json!({"scope":q,"series":series}),
        refs,
        pinned,
    )?;
    let selected = manifest
        .selected_run
        .as_ref()
        .and_then(|s| eligible.iter().find(|r| r.run.id == s.run_id));
    let mut prices = selected
        .map(|r| {
            r.prices
                .iter()
                .filter(|p| {
                    p.value.instrument == query.instrument
                        && (query.start..=query.end).contains(&p.value.date)
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    prices.sort_by(|a, b| {
        a.value
            .date
            .cmp(&b.value.date)
            .then(a.retrieval.fingerprint.cmp(&b.retrieval.fingerprint))
            .then(a.retrieval.observation_id.cmp(&b.retrieval.observation_id))
    });
    let mut conflicts = vec![];
    for pair in prices.windows(2) {
        if pair[0].value.date == pair[1].value.date {
            conflicts.push(format!(
                "Multiple observations for trading date {}",
                pair[0].value.date
            ));
        }
        if pair[0].value.currency != pair[1].value.currency
            || pair[0].value.exchange_timezone != pair[1].value.exchange_timezone
            || pair[0].value.price_basis != pair[1].value.price_basis
        {
            conflicts.push("Series currency, timezone or price basis changed".into());
        }
    }
    conflicts.sort();
    conflicts.dedup();
    manifest.observations = prices
        .iter()
        .map(|e| reference(e, &e.value.source_url))
        .collect();
    let values = if conflicts.is_empty() {
        prices
            .iter()
            .map(|e| match series {
                PriceSeries::Close => Some(e.value.close.clone()),
                PriceSeries::AdjustedClose => e.value.adjusted_close.clone(),
            })
            .collect()
    } else {
        vec![]
    };
    let coverage = selected.and_then(|r| r.run.coverage.clone());
    manifest.source_coverage = coverage.clone();
    Ok(DailySelection {
        manifest,
        series,
        prices,
        values,
        conflicts,
        coverage,
    })
}
pub fn select_facts(
    provider: &ProviderIdentity,
    query: &MetricQuery,
    runs: &[FinancialRunEvidence],
    pinned: Option<i64>,
) -> Result<FactSelection> {
    validate_provider(provider)?;
    query.scope.validate()?;
    if [&query.namespace, &query.concept, &query.unit]
        .iter()
        .any(|s| s.trim().is_empty())
    {
        return Err(Error::new(
            ErrorKind::InvalidRequest,
            "exact concept namespace and unit required",
        ));
    }
    if let PeriodSelection::Exact { period } = &query.periods {
        period.validate()?;
    }
    let eligible: Vec<_> = runs
        .iter()
        .filter(|r| {
            r.run.provider == *provider
                && ["facts", "both"].contains(&r.run.operation.as_str())
                && financial_contains(&r.run.query, &query.scope)
        })
        .collect();
    let mut q = query.clone();
    q.scope.cursor = None;
    q.scope.page_size = 1;
    q.scope.forms.sort();
    q.scope.forms.dedup();
    let refs = eligible
        .iter()
        .map(|r| {
            run_ref(
                &r.context,
                "financial",
                r.run.id,
                r.run.status,
                &r.run.error,
            )
        })
        .collect();
    let mut manifest = manifest(
        "reported-facts:1",
        provider,
        serde_json::to_value(q)?,
        refs,
        pinned,
    )?;
    let selected = manifest
        .selected_run
        .as_ref()
        .and_then(|s| eligible.iter().find(|r| r.run.id == s.run_id));
    let mut grouped: BTreeMap<String, Vec<Evidence<Fact>>> = BTreeMap::new();
    if let Some(run) = selected {
        for e in &run.facts {
            let f = &e.value;
            let period_match = match &query.periods {
                PeriodSelection::LatestInstant | PeriodSelection::Instants => {
                    matches!(f.period, Period::Instant { .. })
                }
                PeriodSelection::Durations => matches!(f.period, Period::Duration { .. }),
                PeriodSelection::Exact { period } => f.period == *period,
            };
            if period_match
                && crate::storage::matches_query(&query.scope, &f.company, f.filed, &f.form)
                && f.namespace == query.namespace
                && f.concept == query.concept
                && f.unit == query.unit
            {
                grouped
                    .entry(serde_json::to_string(&f.period)?)
                    .or_default()
                    .push(e.clone());
            }
        }
    }
    let mut groups = vec![];
    for mut candidates in grouped.into_values() {
        let filed = candidates.iter().map(|e| e.value.filed).max().unwrap();
        candidates.retain(|e| e.value.filed == filed);
        candidates.sort_by(|a, b| {
            a.retrieval
                .fingerprint
                .cmp(&b.retrieval.fingerprint)
                .then(a.retrieval.observation_id.cmp(&b.retrieval.observation_id))
        });
        let key = decimal_key(&candidates[0].value.value);
        let conflict = candidates
            .iter()
            .any(|e| decimal_key(&e.value.value) != key);
        groups.push(FactGroup {
            period: candidates[0].value.period.clone(),
            filed,
            value: if conflict {
                None
            } else {
                Some(Decimal::new(key)?)
            },
            candidates,
            conflict: conflict
                .then(|| "Latest filing date contains disagreeing exact values".into()),
        });
    }
    groups.sort_by_key(|g| match g.period {
        Period::Instant { date } => (date, date),
        Period::Duration { start, end } => (end, start),
    });
    if matches!(query.periods, PeriodSelection::LatestInstant) && groups.len() > 1 {
        groups = groups.split_off(groups.len() - 1);
    }
    manifest.observations = groups
        .iter()
        .flat_map(|g| {
            g.candidates
                .iter()
                .map(|e| reference(e, &e.value.source_url))
        })
        .collect();
    Ok(FactSelection { manifest, groups })
}
