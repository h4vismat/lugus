use lugus_financial::{
    domain::{Decimal, Validate},
    market_data::*,
};
use serde_json::json;

fn bar() -> PriceBar {
    serde_json::from_value(json!({"instrument":{"namespace":"yahoo:symbol","value":"AAPL"},"date":"2024-01-02","open":"10000000000000000000.01","high":"10000000000000000000.03","low":"10000000000000000000.00","close":"10000000000000000000.02","volume":18446744073709551615u64,"adjusted_close":"99.5","currency":"USD","exchange_timezone":"America/New_York","price_basis":"source_reported","precision":"binary_float_source","source_url":"https://finance.yahoo.com/quote/AAPL/history/","retrieved_at":"2026-09-09T00:00:00Z"})).unwrap()
}
fn query() -> PriceQuery {
    serde_json::from_value(json!({"instrument":{"namespace":"yahoo:symbol","value":"AAPL"},"start":"2024-01-01","end":"2024-01-31","cursor":null,"page_size":100})).unwrap()
}
#[test]
fn exact_ohlc_validation_does_not_round_through_floats() {
    let mut b = bar();
    b.validate().unwrap();
    b.close = Decimal::new("10000000000000000000.04").unwrap();
    assert!(b.validate().is_err());
    b.close = Decimal::new("-1").unwrap();
    assert!(b.validate().is_err());
}
#[test]
fn page_requires_scoped_sorted_bars_and_consistent_coverage() {
    let mut page = PricePage {
        items: vec![bar()],
        next_cursor: None,
        coverage: PriceCoverage {
            first_date: Some("2024-01-02".parse().unwrap()),
            last_date: Some("2024-01-02".parse().unwrap()),
            completeness: Completeness::Unverified,
        },
    };
    page.validate_for(&query()).unwrap();
    page.items.push(bar());
    assert!(page.validate_for(&query()).is_err());
    page.items.pop();
    page.items[0].instrument.value = "MSFT".into();
    assert!(page.validate_for(&query()).is_err());
    page.items[0] = bar();
    page.coverage.last_date = Some("2024-02-01".parse().unwrap());
    assert!(page.validate_for(&query()).is_err());
}
#[test]
fn revisions_preserve_values_but_ignore_retrieval_clock() {
    let a = bar();
    let mut b = a.clone();
    b.retrieved_at = "2026-09-10T00:00:00Z".parse().unwrap();
    assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
    b.adjusted_close = Some(Decimal::new("99.6").unwrap());
    assert_ne!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
}
#[test]
fn numeric_wire_prices_and_fractional_volume_are_rejected() {
    let mut value = serde_json::to_value(bar()).unwrap();
    value["open"] = json!(1.25);
    assert!(serde_json::from_value::<PriceBar>(value).is_err());
    let mut value = serde_json::to_value(bar()).unwrap();
    value["volume"] = json!(1.5);
    assert!(serde_json::from_value::<PriceBar>(value).is_err());
}
