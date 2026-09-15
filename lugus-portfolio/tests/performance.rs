use lugus_portfolio::*;
use serde_json::json;
fn d(s: &str) -> Decimal {
    Decimal::parse(s).unwrap()
}
#[test]
fn deposits_and_withdrawals_are_not_gains() {
    assert_eq!(
        daily_growth(&d("100"), &d("150"), &d("50"), &d("0")).unwrap(),
        Some(d("1"))
    );
    assert_eq!(
        daily_growth(&d("100"), &d("110"), &d("0"), &d("10")).unwrap(),
        Some(d("1.2"))
    );
    assert_eq!(
        daily_growth(&d("0"), &d("0"), &d("0"), &d("0")).unwrap(),
        None
    );
}
#[test]
fn two_ten_percent_days_compound_and_benchmark_is_independent() {
    let rows:Vec<DailyValuation>=serde_json::from_value(json!([
        {"date":"2026-01-01","value":"100","deposits":"0","withdrawals":"0","opening_contribution":"0","benchmark_level":"1000","observations":[],"issues":[]},
        {"date":"2026-01-02","value":"110","deposits":"0","withdrawals":"0","opening_contribution":"0","benchmark_level":"1010","observations":[],"issues":[]},
        {"date":"2026-01-03","value":"121","deposits":"0","withdrawals":"0","opening_contribution":"0","benchmark_level":"1020","observations":[],"issues":[]}
    ])).unwrap();
    let series = calculate_performance(&rows).unwrap();
    assert_eq!(series.summary.portfolio_return_percent, Some(d("21")));
    assert_eq!(series.summary.benchmark_return_percent, Some(d("2")));
    assert_eq!(series.summary.difference_pp, Some(d("19")));
    let mut gap = rows.clone();
    gap[1].value = None;
    let series = calculate_performance(&gap).unwrap();
    assert_eq!(series.summary.portfolio_return_percent, None);
    assert_eq!(series.summary.benchmark_return_percent, Some(d("2")));
    assert_eq!(series.points[2].value, Some(d("121")));
}
#[test]
fn total_loss_recapitalization_starts_a_separate_segment() {
    let rows:Vec<DailyValuation>=serde_json::from_value(json!([
        {"date":"2026-01-01","value":"100","deposits":"0","withdrawals":"0","opening_contribution":"0","benchmark_level":"100","observations":[],"issues":[]},
        {"date":"2026-01-02","value":"0","deposits":"0","withdrawals":"0","opening_contribution":"0","benchmark_level":"100","observations":[],"issues":[]},
        {"date":"2026-01-03","value":"55","deposits":"50","withdrawals":"0","opening_contribution":"0","benchmark_level":"100","observations":[],"issues":[]}
    ])).unwrap();
    let result = calculate_performance(&rows).unwrap();
    assert_eq!(result.points[1].portfolio_return_percent, Some(d("-100")));
    assert_eq!(result.points[2].segment, 1);
    assert_eq!(result.points[2].segment_return_percent, Some(d("10")));
    assert_eq!(result.summary.portfolio_return_percent, None);
}
#[test]
fn withdrawals_can_exhaust_the_hypothetical_without_hiding_index_return() {
    let rows:Vec<DailyValuation>=serde_json::from_value(json!([
        {"date":"2026-01-01","value":"100","deposits":"0","withdrawals":"0","opening_contribution":"0","benchmark_level":"100","observations":[],"issues":[]},
        {"date":"2026-01-02","value":"30","deposits":"0","withdrawals":"120","opening_contribution":"0","benchmark_level":"100","observations":[],"issues":[]}
    ])).unwrap();
    let result = calculate_performance(&rows).unwrap();
    assert_eq!(result.summary.portfolio_return_percent, Some(d("50")));
    assert_eq!(result.summary.benchmark_return_percent, Some(d("0")));
    assert_eq!(result.points[1].hypothetical_value, None);
    assert!(
        result.points[1]
            .issues
            .iter()
            .any(|i| i.code == HistoryIssueCode::BenchmarkOverdraft)
    );
}
#[test]
fn empty_capital_can_be_funded_and_full_withdrawal_is_not_a_total_loss() {
    let rows:Vec<DailyValuation>=serde_json::from_value(json!([
        {"date":"2026-01-01","value":"0","deposits":"0","withdrawals":"0","opening_contribution":"0","benchmark_level":"100","observations":[],"issues":[]},
        {"date":"2026-01-02","value":"100","deposits":"100","withdrawals":"0","opening_contribution":"0","benchmark_level":"100","observations":[],"issues":[]},
        {"date":"2026-01-03","value":"0","deposits":"0","withdrawals":"100","opening_contribution":"0","benchmark_level":"100","observations":[],"issues":[]},
        {"date":"2026-01-04","value":"110","deposits":"100","withdrawals":"0","opening_contribution":"0","benchmark_level":"100","observations":[],"issues":[]}
    ])).unwrap();
    let result = calculate_performance(&rows).unwrap();
    assert_eq!(result.points[0].portfolio_return_percent, None);
    assert_eq!(result.points[3].segment, 0);
    assert_eq!(result.summary.portfolio_return_percent, Some(d("10")));
    assert_eq!(
        calculate_performance(&rows[..1])
            .unwrap()
            .summary
            .portfolio_return_percent,
        None
    );
}
