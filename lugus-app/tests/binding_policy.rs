use lugus_app::bindings::{BindingAssessment, assess_binding};
use lugus_financial::{
    domain::CompanyId,
    instruments::{InstrumentKind, InstrumentMetadata},
    resolution::{Candidate, Listing},
};
fn id(namespace: &str, value: &str) -> CompanyId {
    CompanyId {
        namespace: namespace.into(),
        value: value.into(),
    }
}
fn fixture() -> (Candidate, Listing, InstrumentMetadata) {
    let listing = Listing {
        ticker: id("sec:ticker", "AAPL"),
        exchange: Some(id("sec:exchange", "NASDAQ")),
    };
    let candidate = Candidate {
        identifier: id("sec:cik", "0000320193"),
        name: "Apple Inc.".into(),
        aliases: vec![],
        listings: vec![listing.clone()],
        source_url: "https://sec.test".into(),
        source_checksum: "a".repeat(64),
        retrieved_at: "2026-09-10T00:00:00Z".parse().unwrap(),
        match_reasons: vec![],
    };
    let metadata = InstrumentMetadata {
        instrument: lugus_financial::market_data::InstrumentId {
            namespace: "yahoo:symbol".into(),
            value: "AAPL".into(),
        },
        issuer_name: Some("APPLE INC".into()),
        ticker: Some(" aapl ".into()),
        exchange: Some(id("yahoo:exchange", "NMS")),
        kind: Some(InstrumentKind::Equity),
        issuer_identifiers: vec![],
        source_url: "https://yahoo.test".into(),
        source_checksum: "b".repeat(64),
        retrieved_at: candidate.retrieved_at,
    };
    (candidate, listing, metadata)
}
#[test]
fn explicit_policy_distinguishes_support_conflict_and_missing_evidence() {
    let (c, l, m) = fixture();
    assert!(matches!(
        assess_binding(&c, &l, &m),
        BindingAssessment::Supported { .. }
    ));
    for (field, value, expected) in [
        ("issuer", "Microsoft Corp", "conflict"),
        ("ticker", "AAP-L", "conflict"),
        ("exchange", "NYQ", "conflict"),
        ("exchange", "UNKNOWN", "incomplete"),
        ("kind", "other", "conflict"),
        ("kind", "missing", "incomplete"),
        ("issuer", "missing", "incomplete"),
        ("ticker", "missing", "incomplete"),
    ] {
        let mut m = m.clone();
        match field {
            "issuer" => m.issuer_name = (value != "missing").then(|| value.into()),
            "ticker" => m.ticker = (value != "missing").then(|| value.into()),
            "exchange" => m.exchange = Some(id("yahoo:exchange", value)),
            "kind" => m.kind = (value != "missing").then_some(InstrumentKind::Other),
            _ => unreachable!(),
        };
        let actual = assess_binding(&c, &l, &m);
        assert!(
            matches!(
                (&actual, expected),
                (BindingAssessment::Conflict { .. }, "conflict")
                    | (BindingAssessment::Incomplete { .. }, "incomplete")
            ),
            "{field}: {actual:?}"
        );
    }
}
#[test]
fn shared_identifier_agreement_does_not_override_other_conflicts() {
    let (c, l, mut m) = fixture();
    m.issuer_name = None;
    m.issuer_identifiers = vec![c.identifier.clone()];
    assert!(matches!(
        assess_binding(&c, &l, &m),
        BindingAssessment::Supported { .. }
    ));
    m.issuer_identifiers.push(id("sec:cik", "0000789019"));
    assert!(matches!(
        assess_binding(&c, &l, &m),
        BindingAssessment::Conflict { .. }
    ));
}
#[test]
fn selected_listing_requires_exact_membership_and_names_keep_legal_words() {
    let (mut c, l, mut m) = fixture();
    c.listings.push(Listing {
        ticker: id("sec:ticker", "AAPL.B"),
        exchange: l.exchange.clone(),
    });
    let mut absent = l.clone();
    absent.ticker.value = "aapl".into();
    assert!(matches!(
        assess_binding(&c, &absent, &m),
        BindingAssessment::Conflict { .. }
    ));
    m.issuer_name = Some("Apple".into());
    assert!(matches!(
        assess_binding(&c, &l, &m),
        BindingAssessment::Conflict { .. }
    ));
    c.aliases.push("Apple".into());
    assert!(matches!(
        assess_binding(&c, &l, &m),
        BindingAssessment::Supported { .. }
    ));
}
