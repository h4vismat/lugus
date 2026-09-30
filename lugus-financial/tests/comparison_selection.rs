#[path = "support/comparison.rs"]
mod support;
use lugus_financial::{comparison::*, domain::*};
use support::*;
#[test]
fn groups_actual_periods_and_is_order_independent() {
    let mut facts = vec![
        revenue(1, 2021, "80"),
        revenue(2, 2022, "90"),
        revenue(3, 2023, "100"),
        revenue(4, 2024, "120"),
    ];
    let mut repeat = revenue(5, 2023, "100.0");
    repeat.evidence.value.fiscal_year = Some(2025);
    facts.push(repeat);
    let a = selected(&facts);
    assert_eq!(a.periods.len(), 4);
    assert_eq!(a.periods[2].revenue.value.as_ref().unwrap().as_str(), "100");
    assert_eq!(a.periods[2].revenue.inputs.len(), 2);
    facts.reverse();
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(selected(&facts)).unwrap()
    );
}
#[test]
fn preserves_latest_filing_conflicts() {
    let mut old = revenue(1, 2024, "90");
    old.evidence.value.filed = "2025-01-01".parse().unwrap();
    let mut other = revenue(3, 2024, "101");
    other.evidence.value.form = "10-K/A".into();
    let a = selected(&[old, revenue(2, 2024, "100"), other]);
    let cell = &a.periods[0].revenue;
    assert!(cell.value.is_none());
    assert_eq!(cell.inputs.len(), 2);
    assert!(cell.issues.iter().any(|i| i.code == "conflicting_values"));
}
#[test]
fn eligibility_boundaries_are_explicit() {
    for (days, eligible) in [(349, false), (350, true), (380, true), (381, false)] {
        let mut f = revenue(1, 2024, "1");
        let end = "2024-12-31".parse::<chrono::NaiveDate>().unwrap();
        f.evidence.value.period = Period::Duration {
            start: end - chrono::Duration::days(days - 1),
            end,
        };
        assert_eq!(!selected(&[f]).periods.is_empty(), eligible, "{days}");
    }
    for alteration in 0..5 {
        let mut f = revenue(1, 2024, "1");
        match alteration {
            0 => f.evidence.value.form = "10-Q".into(),
            1 => f.evidence.value.fiscal_period = None,
            2 => f.evidence.value.unit = "EUR".into(),
            3 => f.evidence.value.namespace = "ifrs-full".into(),
            _ => f = revenue(1, 2025, "1"),
        };
        let a = selected(&[f]);
        assert!(a.periods.is_empty());
        assert!(!a.issues.is_empty());
    }
}
#[test]
fn refuses_substitution_and_cross_company_inputs() {
    let a = selected(&[
        fact(1, 2024, "Revenues", "100"),
        fact(2, 2024, "NetIncomeLoss", "10"),
    ]);
    assert!(a.periods[0].revenue.value.is_none());
    let mut wrong = revenue(1, 2024, "100");
    wrong.evidence.value.company.value = "2".into();
    assert!(select_annual(&company(), &[wrong], &policy()).is_err());
    for years in [0, 6] {
        let mut p = policy();
        p.years = years;
        assert!(select_annual(&company(), &[], &p).is_err());
    }
    assert!(!selected(&[]).issues.is_empty());
}
#[test]
fn does_not_pick_an_arbitrary_duration() {
    let a = revenue(1, 2024, "100");
    let mut b = revenue(2, 2024, "100");
    b.evidence.value.period = Period::Duration {
        start: "2024-01-02".parse().unwrap(),
        end: "2024-12-31".parse().unwrap(),
    };
    let s = selected(&[a, b]);
    assert!(s.periods[0].start.is_none());
    assert!(s.periods[0].revenue.value.is_none());
    assert_eq!(s.periods[0].candidate_periods.len(), 2);
}
