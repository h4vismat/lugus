#[path = "support/comparison.rs"]
mod support;
use lugus_financial::{comparison::*, domain::*};
use support::*;
fn rows(rev: &str, income: &str, prior: &str) -> Vec<AnnualRow> {
    calculate_annual(
        &selected(&[
            revenue(1, 2023, prior),
            revenue(2, 2024, rev),
            fact(3, 2024, "NetIncomeLoss", income),
        ]),
        &policy(),
    )
    .unwrap()
}
#[test]
fn calculates_exact_percentages_and_independent_rounding() {
    let r = rows("120", "-15", "100");
    let r = r.last().unwrap();
    assert_eq!(r.revenue_growth.result.as_ref().unwrap().display, "20.00");
    assert_eq!(
        r.net_margin.result.as_ref().unwrap().value.as_str(),
        "-12.5"
    );
    for (r, n, want) in [
        ("32", "1", "3.12"),
        ("32", "3", "9.38"),
        ("100000000000", "2344999999", "2.34"),
    ] {
        let a = rows(r, n, "100");
        assert_eq!(
            a.last()
                .unwrap()
                .net_margin
                .result
                .as_ref()
                .unwrap()
                .display,
            want
        );
    }
    let a = rows("100000000000", "2344999999", "100");
    assert_eq!(
        a.last()
            .unwrap()
            .net_margin
            .result
            .as_ref()
            .unwrap()
            .value
            .as_str(),
        "2.345"
    );
    let a = rows("9007199254740993", "1", "9007199254740992");
    assert_eq!(
        a.last()
            .unwrap()
            .period
            .revenue
            .value
            .as_ref()
            .unwrap()
            .as_str(),
        "9007199254740993"
    );
    assert_eq!(
        a.last()
            .unwrap()
            .revenue_growth
            .result
            .as_ref()
            .unwrap()
            .numerator,
        "25"
    );
    assert_eq!(
        a.last()
            .unwrap()
            .revenue_growth
            .result
            .as_ref()
            .unwrap()
            .denominator,
        "2251799813685248"
    );
}
#[test]
fn undefined_is_not_zero_and_numeric_limits_preserve_raw() {
    for value in ["0", "-1"] {
        let a = rows(value, "1", value);
        let r = a.last().unwrap();
        assert!(r.net_margin.result.is_none());
        assert!(r.revenue_growth.result.is_none());
        assert!(!r.net_margin.inputs.is_empty());
    }
    for value in ["9".repeat(129), "1.1234567890123456789".into()] {
        let a = rows(&value, "1", "100");
        let r = a.last().unwrap();
        assert_eq!(r.period.revenue.value.as_ref().unwrap().as_str(), value);
        assert!(
            r.net_margin
                .issues
                .iter()
                .any(|i| i.code == "numeric_limit")
        );
    }
}
#[test]
fn requires_consecutive_unambiguous_periods_and_common_disclosure() {
    let mut income = fact(3, 2024, "NetIncomeLoss", "10");
    income.evidence.value.filing_id = "other-filing".into();
    let a = calculate_annual(
        &selected(&[revenue(1, 2022, "80"), revenue(2, 2024, "100"), income]),
        &policy(),
    )
    .unwrap();
    let r = a.last().unwrap();
    assert!(r.revenue_growth.result.is_none());
    assert!(r.net_margin.result.is_none());
    let a = calculate_annual(
        &selected(&[
            revenue(1, 2023, "80"),
            revenue(2, 2024, "100"),
            revenue(3, 2024, "110"),
        ]),
        &policy(),
    )
    .unwrap();
    assert!(a.last().unwrap().revenue_growth.result.is_none());
}
#[test]
fn warns_about_period_length_without_annualizing() {
    let mut prior = revenue(1, 2023, "100");
    prior.evidence.value.period = Period::Duration {
        start: "2023-01-01".parse().unwrap(),
        end: "2023-12-30".parse().unwrap(),
    };
    let mut current = revenue(2, 2024, "120");
    current.evidence.value.period = Period::Duration {
        start: "2023-12-31".parse().unwrap(),
        end: "2025-01-04".parse().unwrap(),
    };
    let mut p = policy();
    p.period_end = "2025-01-04".parse().unwrap();
    let s = select_annual(&company(), &[prior, current], &p).unwrap();
    let a = calculate_annual(&s, &p).unwrap();
    let r = a.last().unwrap();
    assert_eq!(r.revenue_growth.result.as_ref().unwrap().display, "20.00");
    assert!(
        r.revenue_growth
            .issues
            .iter()
            .any(|i| i.code == "period_length_difference")
    );
}
#[test]
fn original_numeric_limits_survive_equivalent_canonical_values() {
    for raw in [
        format!("{}1", "0".repeat(128)),
        "1.0000000000000000000".into(),
    ] {
        for duplicate in [false, true] {
            let mut facts = vec![
                revenue(1, 2023, "1"),
                revenue(2, 2024, &raw),
                fact(3, 2024, "NetIncomeLoss", "1"),
            ];
            if duplicate {
                facts.push(revenue(4, 2024, "1"));
            }
            let selected = selected(&facts);
            let result = calculate_annual(&selected, &policy()).unwrap();
            let row = result.last().unwrap();
            assert!(row.net_margin.result.is_none(), "{raw}");
            assert_eq!(row.net_margin.issues[0].code, "numeric_limit");
            assert_eq!(row.revenue_growth.issues[0].code, "numeric_limit");
            assert_eq!(row.period.revenue.value.as_ref().unwrap().as_str(), raw);
        }
    }
}
