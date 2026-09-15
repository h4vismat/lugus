//! Daily ledger values with explicit source gaps and per-account action reconciliation.
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricalClose {
    pub instrument_id: String,
    pub date: Day,
    pub session_close: Option<chrono::DateTime<chrono::Utc>>,
    pub close: Option<Decimal>,
    pub split: Option<(u64, u64)>,
    pub observation_id: String,
    pub unsupported_action: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryIssue {
    pub code: HistoryIssueCode,
    pub date: Day,
    pub instrument_id: Option<String>,
    pub account_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryIssueCode {
    MissingPrice,
    MissingBaseline,
    UnknownCalendar,
    IncompatiblePrice,
    SplitMismatch,
    UnsupportedAction,
    MissingBenchmark,
    BenchmarkSelectionRequired,
    ZeroCapital,
    TotalLossRestart,
    BenchmarkOverdraft,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyValuation {
    pub date: Day,
    pub value: Option<Decimal>,
    pub deposits: Option<Decimal>,
    pub withdrawals: Decimal,
    pub opening_contribution: Option<Decimal>,
    pub benchmark_level: Option<Decimal>,
    pub observations: Vec<String>,
    pub issues: Vec<HistoryIssue>,
}
pub struct ValuationHistoryInput<'a> {
    pub ledgers: &'a [Ledger],
    pub closes: &'a [HistoricalClose],
    pub benchmark: &'a [HistoricalClose],
    pub baseline: Day,
    pub end: Day,
}
type Prices<'a> = BTreeMap<(&'a str, Day), (Option<&'a HistoricalClose>, &'a HistoricalClose)>;
fn prices(rows: &[HistoricalClose]) -> Result<Prices<'_>> {
    let mut ordered = BTreeMap::new();
    for row in rows {
        require(
            !row.instrument_id.is_empty() && !row.observation_id.is_empty(),
            "historical observation requires identity",
        )?;
        require(
            row.close.as_ref().is_none_or(|v| !v.is_negative()),
            "negative historical close",
        )?;
        require(
            row.session_close.is_some() || (row.close.is_none() && row.split.is_none()),
            "price or action on a calendar closure",
        )?;
        require(
            row.split.is_none_or(|(n, d)| n > 0 && d > 0),
            "invalid historical split",
        )?;
        require(
            ordered
                .insert((row.instrument_id.as_str(), row.date), row)
                .is_none(),
            "duplicate historical observation",
        )?;
    }
    let mut out = Prices::new();
    for ((id, date), row) in ordered {
        let observed = if row.session_close.is_some() {
            row.close.as_ref().map(|_| row)
        } else {
            date.pred_opt()
                .and_then(|prev| out.get(&(id, prev)))
                .and_then(|(price, _)| *price)
        };
        out.insert((id, date), (observed, row));
    }
    Ok(out)
}
fn issue(
    code: HistoryIssueCode,
    date: Day,
    id: Option<&str>,
    account: Option<&str>,
) -> HistoryIssue {
    HistoryIssue {
        code,
        date,
        instrument_id: id.map(str::to_owned),
        account_id: account.map(str::to_owned),
    }
}
fn add(total: &mut Option<Decimal>, value: Option<Decimal>) -> Result<()> {
    *total = match (total.take(), value) {
        (Some(a), Some(b)) => Some(a.checked_add(&b)?),
        _ => None,
    };
    Ok(())
}
fn quantities(lots: &[Lot]) -> Result<BTreeMap<&str, Decimal>> {
    let mut out = BTreeMap::new();
    for lot in lots {
        let v = out
            .entry(lot.instrument_id.as_str())
            .or_insert_with(Decimal::zero);
        *v = v.checked_add(&lot.quantity)?;
    }
    Ok(out)
}
fn holding_value(
    id: &str,
    quantity: &Decimal,
    date: Day,
    account: &str,
    prices: &Prices<'_>,
    bad: &BTreeMap<String, HistoryIssueCode>,
    day: &mut DailyValuation,
) -> Result<Option<Decimal>> {
    if quantity.is_zero() {
        return Ok(Some(Decimal::zero()));
    }
    let found = prices.get(&(id, date));
    let problem = bad.get(id).cloned().or_else(|| match found {
        None => Some(HistoryIssueCode::UnknownCalendar),
        Some((None, _)) => Some(HistoryIssueCode::MissingPrice),
        _ => None,
    });
    if let Some(code) = problem {
        day.issues
            .push(issue(code, day.date, Some(id), Some(account)));
        return Ok(None);
    }
    let (Some(observed), calendar) = found.unwrap() else {
        unreachable!()
    };
    day.observations.push(observed.observation_id.clone());
    day.observations.push(calendar.observation_id.clone());
    Ok(Some(
        quantity.checked_mul(observed.close.as_ref().unwrap())?,
    ))
}
fn same_ratio(a: (u64, u64), b: (u64, u64)) -> bool {
    u128::from(a.0) * u128::from(b.1) == u128::from(b.0) * u128::from(a.1)
}
pub fn historical_values(input: &ValuationHistoryInput<'_>) -> Result<Vec<DailyValuation>> {
    historical_values_cancellable(input, &|| false)
}
pub fn historical_values_cancellable(
    input: &ValuationHistoryInput<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<DailyValuation>> {
    require(
        input.baseline <= input.end && (input.end - input.baseline).num_days() < 100_000,
        "invalid historical valuation range",
    )?;
    require(
        input.closes.len() <= 100_000 && input.benchmark.len() <= 100_000,
        "historical observation limit exceeded",
    )?;
    let portfolio_prices = prices(input.closes)?;
    let benchmark = prices(input.benchmark)?;
    let benchmark_ids: BTreeSet<_> = input
        .benchmark
        .iter()
        .map(|r| r.instrument_id.as_str())
        .collect();
    require(benchmark_ids.len() <= 1, "ambiguous benchmark")?;
    let benchmark_id = benchmark_ids.first().copied();
    let mut ids = BTreeSet::new();
    let mut accounts = Vec::new();
    for ledger in input.ledgers {
        require(ids.insert(&ledger.account_id), "duplicate account")?;
        let mut events = BTreeMap::<Day, Vec<&Event>>::new();
        for e in &ledger.events {
            events.entry(e.date).or_default().push(e);
        }
        accounts.push((
            ledger,
            ReplayCursor::new(ledger)?,
            BTreeMap::<String, HistoryIssueCode>::new(),
            events,
        ));
    }
    let earliest = input
        .ledgers
        .iter()
        .map(|l| l.start)
        .min()
        .unwrap_or(input.baseline)
        .min(input.baseline);
    require(
        (input.end - earliest).num_days() < 100_000,
        "account history range exceeds limit",
    )?;
    let mut output = Vec::new();
    let mut issue_count = 0usize;
    let mut date = earliest;
    loop {
        if cancelled() {
            return Err(PortfolioError::Cancelled);
        }
        let mut day = DailyValuation {
            date,
            value: Some(Decimal::zero()),
            deposits: Some(Decimal::zero()),
            withdrawals: Decimal::zero(),
            opening_contribution: Some(Decimal::zero()),
            benchmark_level: None,
            observations: vec![],
            issues: vec![],
        };
        for (ledger, cursor, bad, events) in &mut accounts {
            if date < ledger.start {
                continue;
            }
            // The cursor still holds the prior day's lots, or the pre-event opening lots.
            let before = quantities(&cursor.state().lots)?
                .into_iter()
                .map(|(id, q)| (id.to_owned(), q))
                .collect::<BTreeMap<_, _>>();
            if date == ledger.start {
                if let Opening::Existing { cash, lots } = &ledger.opening {
                    let mut contribution = Some(cash.clone());
                    for lot in lots {
                        let value = match date.pred_opt() {
                            Some(prior) => holding_value(
                                &lot.instrument_id,
                                &lot.quantity,
                                prior,
                                &ledger.account_id,
                                &portfolio_prices,
                                bad,
                                &mut day,
                            )?,
                            None => None,
                        };
                        add(&mut contribution, value)?;
                    }
                    add(&mut day.opening_contribution, contribution)?;
                }
            }
            let mut ledger_splits = BTreeMap::<&str, Vec<(u64, u64)>>::new();
            for event in events.get(&date).into_iter().flatten() {
                match &event.kind {
                    EventKind::Deposit { amount } => add(&mut day.deposits, Some(amount.clone()))?,
                    EventKind::Withdrawal { amount } => {
                        day.withdrawals = day.withdrawals.checked_add(amount)?
                    }
                    EventKind::Split {
                        instrument_id,
                        numerator,
                        denominator,
                        ..
                    } => ledger_splits
                        .entry(instrument_id)
                        .or_default()
                        .push((*numerator, *denominator)),
                    _ => {}
                }
            }
            for (id, q) in &before {
                if q.is_zero() {
                    continue;
                }
                let source = portfolio_prices.get(&(id.as_str(), date)).map(|(_, r)| *r);
                let source_split = source.and_then(|r| r.split);
                let recorded = ledger_splits.get(id.as_str());
                let matching = match (source_split, recorded) {
                    (None, None) => true,
                    (Some(a), Some(b)) if b.len() == 1 => same_ratio(a, b[0]),
                    _ => false,
                };
                if !matching {
                    bad.insert(id.clone(), HistoryIssueCode::SplitMismatch);
                }
                if source.is_some_and(|r| r.unsupported_action.is_some()) {
                    bad.insert(id.clone(), HistoryIssueCode::UnsupportedAction);
                }
            }
            let state = cursor.advance_to_cancellable(date, cancelled)?;
            let mut value = Some(state.cash.clone());
            for (id, q) in quantities(&state.lots)? {
                add(
                    &mut value,
                    holding_value(
                        id,
                        &q,
                        date,
                        &ledger.account_id,
                        &portfolio_prices,
                        bad,
                        &mut day,
                    )?,
                )?;
            }
            add(&mut day.value, value)?;
        }
        if let Some(id) = benchmark_id {
            if let Some((Some(observed), calendar)) = benchmark.get(&(id, date)) {
                if observed.close.as_ref().is_some_and(Decimal::is_positive)
                    && calendar.unsupported_action.is_none()
                {
                    day.benchmark_level = observed.close.clone();
                    day.observations.push(observed.observation_id.clone());
                    day.observations.push(calendar.observation_id.clone());
                }
            }
        }
        if day.benchmark_level.is_none() {
            day.issues
                .push(issue(HistoryIssueCode::MissingBenchmark, date, None, None));
        }
        day.observations.sort();
        day.observations.dedup();
        if date >= input.baseline {
            issue_count = issue_count.saturating_add(day.issues.len());
            require(
                issue_count <= 100_000,
                "historical coverage notes exceed limit; narrow the range or account",
            )?;
            output.push(day);
        }
        if date == input.end {
            break;
        }
        date = date.succ_opt().ok_or(PortfolioError::Overflow)?;
    }
    Ok(output)
}
