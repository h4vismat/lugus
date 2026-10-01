use super::*;
use crate::{
    domain::*,
    error::{Error, ErrorKind, Result},
    selection::decimal_key,
};
use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet};
fn amount(rows: &[&FactInput], concept: &str, ambiguous: bool) -> SelectedAmount {
    let mut found: Vec<_> = rows
        .iter()
        .copied()
        .filter(|r| r.evidence.value.concept == concept)
        .collect();
    if !ambiguous && let Some(latest) = found.iter().map(|r| r.evidence.value.filed).max() {
        found.retain(|r| r.evidence.value.filed == latest);
    }
    let mut inputs: Vec<_> = found.iter().map(|r| r.reference()).collect();
    inputs.sort();
    inputs.dedup();
    let filing_ids: BTreeSet<_> = found
        .iter()
        .map(|r| r.evidence.value.filing_id.clone())
        .collect();
    let values: BTreeSet<_> = found
        .iter()
        .map(|r| decimal_key(&r.evidence.value.value))
        .collect();
    let mut issues = vec![];
    let value = if ambiguous {
        issues.push(ComparisonIssue::new(
            "ambiguous_period",
            "Multiple annual durations end on this date",
            inputs.clone(),
        ));
        None
    } else if values.len() > 1 {
        issues.push(ComparisonIssue::new(
            "conflicting_values",
            "Latest filing date contains disagreeing values",
            inputs.clone(),
        ));
        None
    } else if let Some(value) = values.first() {
        // Numeric equality must not erase an original operand's precision limit.
        let oversized = found
            .iter()
            .map(|r| &r.evidence.value.value)
            .filter(|v| {
                v.as_str().len() > 128 || v.as_str().split('.').nth(1).map_or(0, str::len) > 18
            })
            .min_by_key(|v| v.as_str());
        if let Some(raw) = oversized {
            issues.push(ComparisonIssue::new(
                "numeric_limit",
                "An original reported operand exceeds supported calculation precision",
                inputs.clone(),
            ));
            Some(raw.clone())
        } else {
            Some(Decimal::new(value.clone()).expect("canonical validated decimal"))
        }
    } else {
        issues.push(ComparisonIssue::new(
            "missing_data",
            "No eligible observation for the selected concept",
            vec![],
        ));
        None
    };
    SelectedAmount {
        value,
        inputs,
        filing_ids: filing_ids.into_iter().collect(),
        issues,
    }
}
pub fn select_annual(
    company: &CompanyId,
    facts: &[FactInput],
    policy: &AnnualPolicy,
) -> Result<AnnualSelection> {
    company.validate()?;
    policy.validate()?;
    let mut groups: BTreeMap<NaiveDate, Vec<&FactInput>> = BTreeMap::new();
    let mut excluded: BTreeMap<&str, Vec<InputRef>> = BTreeMap::new();
    for row in facts {
        let f = &row.evidence.value;
        f.validate()?;
        if f.company != *company {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "comparison contains another company's evidence",
            ));
        }
        if ![policy.revenue_basis.concept(), "NetIncomeLoss"].contains(&f.concept.as_str()) {
            continue;
        }
        let eligible = if let Period::Duration { start, end } = f.period {
            f.namespace == "us-gaap"
                && f.unit == "USD"
                && ["10-K", "10-K/A"].contains(&f.form.as_str())
                && f.fiscal_period.as_deref() == Some("FY")
                && (350..=380).contains(&((end - start).num_days() + 1))
                && end <= policy.period_end
        } else {
            false
        };
        if !eligible {
            excluded
                .entry("unsupported_period")
                .or_default()
                .push(row.reference());
            continue;
        }
        if let Period::Duration { end, .. } = f.period {
            groups.entry(end).or_default().push(row);
        }
    }
    let mut issues:Vec<_>=excluded.into_iter().map(|(code,refs)|ComparisonIssue::new(code,"Excluded observations have unsupported concept context, unit, form, fiscal period or dates",refs)).collect();
    let take = usize::from(policy.years) + 1;
    let mut periods = vec![];
    for (end, rows) in groups.into_iter().rev().take(take) {
        let starts: BTreeSet<_> = rows
            .iter()
            .filter_map(|r| match r.evidence.value.period {
                Period::Duration { start, .. } => Some(start),
                _ => None,
            })
            .collect();
        let ambiguous = starts.len() != 1;
        let start = (!ambiguous).then(|| *starts.first().unwrap());
        let period_issues = if ambiguous {
            vec![ComparisonIssue::new(
                "ambiguous_period",
                "No single annual duration can be selected",
                rows.iter().map(|r| r.reference()).collect(),
            )]
        } else {
            vec![]
        };
        periods.push(AnnualPeriod {
            start,
            end,
            days: start.map(|s| ((end - s).num_days() + 1) as u16),
            candidate_periods: starts
                .into_iter()
                .map(|start| Period::Duration { start, end })
                .collect(),
            revenue: amount(&rows, policy.revenue_basis.concept(), ambiguous),
            net_income: amount(&rows, "NetIncomeLoss", ambiguous),
            issues: period_issues,
        });
    }
    periods.reverse();
    if periods.is_empty() {
        issues.push(ComparisonIssue::new(
            "missing_data",
            "No eligible annual observations were captured",
            vec![],
        ));
    }
    Ok(AnnualSelection {
        company: company.clone(),
        periods,
        issues,
    })
}
