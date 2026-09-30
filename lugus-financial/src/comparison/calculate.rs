use super::decimal::{growth_percent, ratio_percent};
use super::*;
use crate::error::Result;
fn cell(
    formula: &str,
    mut inputs: Vec<InputRef>,
    result: std::result::Result<PercentageResult, &str>,
) -> CalculatedCell {
    inputs.sort();
    inputs.dedup();
    let (result, issues) = match result {
        Ok(r) => (Some(r), vec![]),
        Err(code) => (
            None,
            vec![ComparisonIssue::new(
                code,
                match code {
                    "nonpositive_denominator" => "The denominator must be positive",
                    "numeric_limit" => "An operand exceeds supported calculation precision",
                    "incompatible_disclosure" => "Inputs do not share an exact period and filing",
                    "missing_prior_period" => "No eligible consecutive prior annual period",
                    "ambiguous_period" => "Annual period is ambiguous",
                    _ => "Required observations are missing or conflicted",
                },
                inputs.clone(),
            )],
        ),
    };
    CalculatedCell {
        result,
        inputs,
        formula: formula.into(),
        issues,
    }
}
pub fn calculate_annual(
    selection: &AnnualSelection,
    policy: &AnnualPolicy,
) -> Result<Vec<AnnualRow>> {
    policy.validate()?;
    let mut rows = vec![];
    for (i, p) in selection.periods.iter().enumerate().skip(
        selection
            .periods
            .len()
            .saturating_sub(policy.years as usize),
    ) {
        let refs = p
            .revenue
            .inputs
            .iter()
            .chain(&p.net_income.inputs)
            .cloned()
            .collect();
        let margin = if p.start.is_none() {
            Err("ambiguous_period")
        } else if let (Some(rev), Some(income)) = (&p.revenue.value, &p.net_income.value) {
            if p.revenue
                .filing_ids
                .iter()
                .any(|id| p.net_income.filing_ids.contains(id))
            {
                ratio_percent(income, rev)
            } else {
                Err("incompatible_disclosure")
            }
        } else {
            Err("missing_data")
        };
        let prior = i.checked_sub(1).map(|j| &selection.periods[j]);
        let mut growth_refs = p.revenue.inputs.clone();
        if let Some(prior) = prior {
            growth_refs.extend(prior.revenue.inputs.clone());
        }
        let growth = if let Some(prior) = prior {
            if p.start.is_none() || prior.start.is_none() {
                Err("ambiguous_period")
            } else if prior.end.succ_opt() != p.start {
                Err("missing_prior_period")
            } else if let (Some(current), Some(old)) = (&p.revenue.value, &prior.revenue.value) {
                growth_percent(current, old)
            } else {
                Err("missing_data")
            }
        } else {
            Err("missing_prior_period")
        };
        let mut revenue_growth = cell("revenue-growth-percent:1", growth_refs, growth);
        if revenue_growth.result.is_some() && prior.is_some_and(|prev| prev.days != p.days) {
            revenue_growth.issues.push(ComparisonIssue::new(
                "period_length_difference",
                "Annual durations differ; growth is not annualized",
                revenue_growth.inputs.clone(),
            ));
        }
        rows.push(AnnualRow {
            period: p.clone(),
            revenue_growth,
            net_margin: cell("net-margin-percent:1", refs, margin),
        });
    }
    Ok(rows)
}
