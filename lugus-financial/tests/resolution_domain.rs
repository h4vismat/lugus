use lugus_financial::{
    domain::{CompanyId, Validate},
    resolution::*,
};
use serde_json::json;
#[test]
fn cashtags_are_explicit_tickers_and_bare_symbols_keep_fallback() {
    let explicit = parse_input(" $brk-b ").unwrap();
    assert_eq!(
        serde_json::to_value(&explicit.primary).unwrap(),
        json!({"kind":"identifier","identifier":{"namespace":"sec:ticker","value":"BRK-B"},"exchange":null})
    );
    assert!(explicit.fallback.is_none());
    let bare = parse_input("IBM").unwrap();
    assert_eq!(
        bare.fallback,
        Some(SearchQuery::Name { text: "IBM".into() })
    );
    assert_eq!(
        parse_input("International Business Machines")
            .unwrap()
            .primary,
        SearchQuery::Name {
            text: "International Business Machines".into()
        }
    );
    for bad in ["", "$", "$$IBM", "$IBM PLTR", "a\nB"] {
        assert!(parse_input(bad).is_err(), "{bad:?}");
    }
}
#[test]
fn cik_normalization_is_explicit_and_invalid_values_are_rejected() {
    assert_eq!(normalize_cik("51143").unwrap(), "0000051143");
    for bad in ["0", "0000000000", "-1", "12345678901", "abc", "1.0"] {
        assert!(normalize_cik(bad).is_err());
    }
    assert!(
        SearchRequest {
            query: SearchQuery::Name { text: "x".into() },
            page_size: 101,
            cursor: None
        }
        .validate()
        .is_err()
    );
}
fn candidate() -> Candidate {
    serde_json::from_value(json!({"identifier":{"namespace":"sec:cik","value":"0000051143"},"name":"IBM","aliases":[],"listings":[{"ticker":{"namespace":"sec:ticker","value":"IBM"},"exchange":{"namespace":"sec:exchange","value":"NYSE"}}],"source_url":"https://example.com/directory","source_checksum":"a".repeat(64),"retrieved_at":"2026-09-10T00:00:00Z","match_reasons":["exact_identifier"]})).unwrap()
}
#[test]
fn response_matching_cannot_claim_an_unrelated_ticker_or_name() {
    let request = SearchRequest {
        query: parse_input("$PLTR").unwrap().primary,
        page_size: 10,
        cursor: None,
    };
    let page = ResolutionPage {
        items: vec![candidate()],
        next_cursor: None,
        snapshot: "snapshot".into(),
        coverage: "directory".into(),
    };
    assert!(page.validate_for(&request).is_err());
    let request = SearchRequest {
        query: parse_input("$IBM").unwrap().primary,
        ..request
    };
    assert!(page.validate_for(&request).is_ok());
    let mut broken = candidate();
    broken.identifier = CompanyId {
        namespace: "sec:cik".into(),
        value: "51143".into(),
    };
    assert!(broken.validate().is_err());
}
