use lugus_financial::domain::*;
use serde_json::json;

#[test]
fn exact_decimal_strings_reject_float_wire_values_and_invalid_grammar() {
    for value in [
        json!(1.2),
        json!("NaN"),
        json!("1e3"),
        json!(""),
        json!("1."),
        json!("+1"),
    ] {
        assert!(serde_json::from_value::<Decimal>(value).is_err());
    }
    let value: Decimal = serde_json::from_value(json!("12345678901234567890.001")).unwrap();
    assert_eq!(value.as_str(), "12345678901234567890.001");
}

#[test]
fn duration_validation_rejects_reversed_dates() {
    let period: Period =
        serde_json::from_value(json!({"kind":"duration","start":"2024-01-01","end":"2023-01-01"}))
            .unwrap();
    assert!(period.validate().is_err());
}

#[test]
fn mapping_respects_namespace_and_period_kind() {
    let mut fact = fact();
    assert_eq!(map_metric(&fact).unwrap().id, "assets");
    fact.namespace = "custom".into();
    assert!(map_metric(&fact).is_none());
    fact.namespace = "us-gaap".into();
    fact.period = Period::Duration {
        start: "2023-01-01".parse().unwrap(),
        end: "2023-12-31".parse().unwrap(),
    };
    assert!(map_metric(&fact).is_none());
}

#[test]
fn identities_ignore_retrieval_time_but_preserve_disclosure_and_value() {
    let a = fact();
    let mut b = a.clone();
    b.retrieved_at = "2026-09-10T00:00:00Z".parse().unwrap();
    assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
    b.filing_id = "amended".into();
    assert_ne!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
}

fn fact() -> Fact {
    serde_json::from_value(json!({"company":{"namespace":"sec:cik","value":"0000320193"},"namespace":"us-gaap","concept":"Assets","label":null,"value":"100.001","unit":"USD","period":{"kind":"instant","date":"2023-12-31"},"filing_id":"original","form":"10-K","filed":"2024-02-01","fiscal_year":2023,"fiscal_period":"FY","source_url":"https://example.com/facts","retrieved_at":"2026-09-09T00:00:00Z"})).unwrap()
}
