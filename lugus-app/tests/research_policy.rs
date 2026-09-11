use lugus_app::{research::*, *};
use serde_json::json;

fn intent(workflow: Workflow) -> ResearchIntent {
    ResearchIntent {
        workflow,
        subjects: vec![SubjectMention {
            text: "AAPL".into(),
            exchange: None,
        }],
        start: None,
        end: None,
        clarification: None,
    }
}

#[test]
fn policy_dates_are_application_defaults_and_explicit_dates_are_preserved() {
    let today = "2026-09-10".parse().unwrap();
    assert_eq!(
        date_range(&intent(Workflow::Prices), today).unwrap(),
        ("2025-09-10".parse().unwrap(), today)
    );
    assert_eq!(
        date_range(&intent(Workflow::Research), today).unwrap(),
        ("2021-09-10".parse().unwrap(), today)
    );
    let mut request = intent(Workflow::Prices);
    request.start = Some("2024-01-01".parse().unwrap());
    request.end = Some("2024-12-31".parse().unwrap());
    assert_eq!(
        date_range(&request, today).unwrap(),
        (request.start.unwrap(), request.end.unwrap())
    );
}

#[test]
fn literal_cik_mentions_are_normalized_by_application() {
    assert_eq!(
        resolution_input("CIK:320193").unwrap(),
        "sec:cik:0000320193"
    );
    assert_eq!(
        resolution_input("CIK 320193").unwrap(),
        "sec:cik:0000320193"
    );
    assert_eq!(resolution_input("$AAPL").unwrap(), "$AAPL");
    assert!(resolution_input("CIK:bogus").is_err());
}

#[test]
fn source_identity_requires_complete_unambiguous_evidence() {
    let candidate = json!({"company":1,"candidate":{"identifier":{"namespace":"sec:cik","value":"0000320193"},"name":"Apple Inc.","aliases":[],"listings":[],"source_url":"https://example.test","source_checksum":"a".repeat(64),"retrieved_at":"2026-09-10T00:00:00Z","match_reasons":["exact_identifier"]},"provider":{"instance_id":"sec","plugin_id":"sec-edgar","plugin_version":"0.2.0"},"run_id":1,"observation_id":1,"recorded_at":"2026-09-10T00:00:00Z"});
    let entry = serde_json::from_value(candidate).unwrap();
    let rows = vec![DatasetRow::Candidate { entry }];
    assert!(resolve_candidate(Some("resolved"), &rows, None).is_ok());
    assert!(resolve_candidate(Some("incomplete"), &rows, None).is_err());
    assert!(resolve_candidate(Some("identity_conflict"), &rows, None).is_err());
    assert!(
        resolve_candidate(Some("resolved"), &[rows[0].clone(), rows[0].clone()], None).is_err()
    );
    assert!(resolve_candidate(Some("resolved"), &rows, Some(1)).is_err());
}
