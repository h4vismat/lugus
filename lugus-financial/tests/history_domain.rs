use lugus_financial::{
    domain::Validate,
    historical_prices::{HistoryPage, HistoryQuery},
};
use serde_json::json;

fn query() -> HistoryQuery {
    serde_json::from_value(json!({"instrument":{"namespace":"yahoo:symbol","value":"TEST"},
        "start":"2026-01-02","end":"2026-01-05","anchor":"2026-01-05","cursor":null,"page_size":200})).unwrap()
}
fn page() -> HistoryPage {
    serde_json::from_value(json!({"manifest":{
        "instrument":{"namespace":"yahoo:symbol","value":"TEST"},"requested_start":"2026-01-02",
        "requested_end":"2026-01-05","coverage_start":"2025-12-31","anchor":"2026-01-05",
        "last_completed_session":"2026-01-05","currency":"USD","exchange_timezone":"America/New_York",
        "calendar":"NYSE","calendar_version":"5.4.0","normalization_version":1,
        "source_basis":"yahoo_split_adjusted_close","completeness":"unverified","retrieved_at":"2026-01-05T22:00:00Z"},
        "items":[{"date":"2025-12-31","market_close":"2025-12-31T21:00:00Z","source_close":"50",
            "close":"100","factor_to_anchor":"2","split":null,"unsupported_action":null,
            "source_url":"https://example.com/history"}],"next_cursor":"next"})).unwrap()
}
#[test]
fn bounds_and_anchor_are_checked_before_fetching() {
    let mut q = query();
    q.page_size = 201;
    assert!(q.validate().is_err());
    q.page_size = 200;
    q.anchor = "2026-01-01".parse().unwrap();
    assert!(q.validate().is_err());
}
#[test]
fn mismatched_manifest_and_prices_on_closures_are_rejected() {
    let q = query();
    let mut p = page();
    assert!(p.validate_for(&q).is_ok());
    p.manifest.instrument.value = "OTHER".into();
    assert!(p.validate_for(&q).is_err());
    let mut p = page();
    p.items[0].market_close = None;
    assert!(p.validate_for(&q).is_err());
    let mut p = page();
    p.items[0].close = None;
    assert!(p.validate_for(&q).is_err());
}
#[test]
fn incomplete_session_missing_price_and_final_coverage_are_distinct() {
    let q = query();
    let mut p = page();
    p.items[0].close = None;
    p.items[0].source_close = None;
    assert!(p.validate_for(&q).is_ok());
    p.next_cursor = None;
    assert!(p.validate_for(&q).is_err());
    let mut p = page();
    p.items[0].market_close = Some("2026-01-06T21:00:00Z".parse().unwrap());
    assert!(p.validate_for(&q).is_err());
}
