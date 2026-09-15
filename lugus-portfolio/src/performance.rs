//! Scale-18 daily time-weighted returns. External flows never count as gains.
use crate::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformancePoint {
    pub date: Day,
    pub value: Option<Decimal>,
    pub deposits: Option<Decimal>,
    pub withdrawals: Decimal,
    pub opening_contribution: Option<Decimal>,
    pub portfolio_growth: Option<Decimal>,
    pub portfolio_return_percent: Option<Decimal>,
    pub segment_return_percent: Option<Decimal>,
    pub segment: u32,
    pub benchmark_return_percent: Option<Decimal>,
    pub hypothetical_value: Option<Decimal>,
    pub issues: Vec<HistoryIssue>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PerformanceSummary {
    pub portfolio_return_percent: Option<Decimal>,
    pub benchmark_return_percent: Option<Decimal>,
    pub difference_pp: Option<Decimal>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformanceSeries {
    pub baseline: Day,
    pub end: Day,
    pub points: Vec<PerformancePoint>,
    pub summary: PerformanceSummary,
}
fn one() -> Decimal {
    Decimal::parse("1").expect("constant decimal")
}
fn percent(factor: &Decimal) -> Result<Decimal> {
    factor
        .checked_sub(&one())?
        .checked_mul(&Decimal::parse("100")?)
}
fn ratio(n: &Decimal, d: &Decimal) -> Result<Decimal> {
    one().allocated(n, d)
}
fn issue(code: HistoryIssueCode, date: Day) -> HistoryIssue {
    HistoryIssue {
        code,
        date,
        instrument_id: None,
        account_id: None,
    }
}
pub fn daily_growth(
    previous: &Decimal,
    close: &Decimal,
    inflows: &Decimal,
    withdrawals: &Decimal,
) -> Result<Option<Decimal>> {
    require(
        [previous, close, inflows, withdrawals]
            .into_iter()
            .all(|v| !v.is_negative()),
        "negative daily value or external flow",
    )?;
    let denominator = previous.checked_add(inflows)?;
    if denominator.is_zero() {
        return Ok(None);
    }
    Ok(Some(ratio(&close.checked_add(withdrawals)?, &denominator)?))
}
pub fn calculate_performance(days: &[DailyValuation]) -> Result<PerformanceSeries> {
    calculate_performance_cancellable(days, &|| false)
}
pub fn calculate_performance_cancellable(
    days: &[DailyValuation],
    cancelled: &dyn Fn() -> bool,
) -> Result<PerformanceSeries> {
    require(
        !days.is_empty() && days.len() <= 100_000,
        "performance requires a bounded baseline series",
    )?;
    let baseline = &days[0];
    let mut factor = baseline.value.as_ref().map(|_| one());
    let mut segment_factor = factor.clone();
    let mut funded = baseline.value.as_ref().is_some_and(Decimal::is_positive);
    let mut segment: u32 = 0;
    let mut lost = false;
    let mut hypothetical = baseline.value.clone();
    let mut points = Vec::with_capacity(days.len());
    for (index, day) in days.iter().enumerate() {
        if cancelled() {
            return Err(PortfolioError::Cancelled);
        }
        require(
            [
                day.value.as_ref(),
                day.deposits.as_ref(),
                Some(&day.withdrawals),
                day.opening_contribution.as_ref(),
                day.benchmark_level.as_ref(),
            ]
            .into_iter()
            .flatten()
            .all(|v| !v.is_negative()),
            "negative daily value or flow",
        )?;
        let mut issues = day.issues.clone();
        let mut growth = None;
        if index > 0 {
            let previous = &days[index - 1];
            require(
                previous.date.succ_opt() == Some(day.date),
                "performance dates must be consecutive",
            )?;
            let inflows = match (&day.deposits, &day.opening_contribution) {
                (Some(a), Some(b)) => Some(a.checked_add(b)?),
                _ => None,
            };
            if let (Some(prev), Some(close), Some(inflows)) =
                (&previous.value, &day.value, &inflows)
            {
                growth = daily_growth(prev, close, inflows, &day.withdrawals)?;
                if lost && inflows.is_positive() {
                    segment = segment.checked_add(1).ok_or(PortfolioError::Overflow)?;
                    segment_factor = Some(one());
                    factor = None;
                    lost = false;
                    issues.push(issue(HistoryIssueCode::TotalLossRestart, day.date));
                }
                if prev.is_positive() || inflows.is_positive() {
                    funded = true;
                }
                if let Some(g) = &growth {
                    factor = factor.map(|f| f.checked_mul(g)).transpose()?;
                    segment_factor = segment_factor.map(|f| f.checked_mul(g)).transpose()?;
                    if g.is_zero() {
                        lost = true;
                    }
                } else if !prev.is_zero()
                    || !close.is_zero()
                    || !inflows.is_zero()
                    || !day.withdrawals.is_zero()
                {
                    factor = None;
                    segment_factor = None;
                    issues.push(issue(HistoryIssueCode::ZeroCapital, day.date));
                }
            } else {
                factor = None;
                segment_factor = None;
            }
            hypothetical = match (
                hypothetical,
                &inflows,
                &previous.benchmark_level,
                &day.benchmark_level,
            ) {
                (Some(value), Some(inflow), Some(before), Some(after))
                    if before.is_positive() && after.is_positive() =>
                {
                    let next = value
                        .checked_add(inflow)?
                        .checked_mul(&ratio(after, before)?)?
                        .checked_sub(&day.withdrawals)?;
                    if next.is_negative() {
                        issues.push(issue(HistoryIssueCode::BenchmarkOverdraft, day.date));
                        None
                    } else {
                        Some(next)
                    }
                }
                _ => None,
            };
        } else if baseline.value.is_none() {
            issues.push(issue(HistoryIssueCode::MissingBaseline, day.date));
        }
        let benchmark_return_percent = match (&baseline.benchmark_level, &day.benchmark_level) {
            (Some(first), Some(now)) if first.is_positive() && now.is_positive() => {
                Some(percent(&ratio(now, first)?)?)
            }
            _ => None,
        };
        if index == 0 && benchmark_return_percent.is_none() {
            hypothetical = None;
        }
        if !funded {
            issues.push(issue(HistoryIssueCode::ZeroCapital, day.date));
        }
        points.push(PerformancePoint {
            date: day.date,
            value: day.value.clone(),
            deposits: day.deposits.clone(),
            withdrawals: day.withdrawals.clone(),
            opening_contribution: day.opening_contribution.clone(),
            portfolio_growth: growth,
            portfolio_return_percent: if funded {
                factor.as_ref().map(percent).transpose()?
            } else {
                None
            },
            segment_return_percent: if funded {
                segment_factor.as_ref().map(percent).transpose()?
            } else {
                None
            },
            segment,
            benchmark_return_percent,
            hypothetical_value: hypothetical.clone(),
            issues,
        });
    }
    let last = points.last().unwrap();
    let summary = PerformanceSummary {
        portfolio_return_percent: last.portfolio_return_percent.clone(),
        benchmark_return_percent: last.benchmark_return_percent.clone(),
        difference_pp: match (
            &last.portfolio_return_percent,
            &last.benchmark_return_percent,
        ) {
            (Some(a), Some(b)) => Some(a.checked_sub(b)?),
            _ => None,
        },
    };
    Ok(PerformanceSeries {
        baseline: baseline.date,
        end: last.date,
        points,
        summary,
    })
}
