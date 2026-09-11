use chrono::NaiveDate;
use lugus_app::research::{ResearchIntent, SubjectMention, Workflow, parse_intent};

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 10).unwrap()
}
fn research(text: &str) -> ResearchIntent {
    ResearchIntent {
        workflow: Workflow::Research,
        subjects: vec![SubjectMention {
            text: text.into(),
            exchange: None,
        }],
        start: None,
        end: None,
        clarification: None,
    }
}

#[test]
fn accepts_literal_mentions_without_source_identity_inference() {
    for text in [
        "Apple Inc.",
        "$AAPL",
        "CIK:0000320193",
        "CIK 320193",
        "Berkshire Hathaway",
    ] {
        let intent = research(text);
        let json = serde_json::to_string(&intent).unwrap();
        assert_eq!(parse_intent(&json, today()).unwrap(), intent);
    }
}

#[test]
fn rejects_untrusted_schema_and_identifier_smuggling() {
    for text in [
        "",
        " ",
        "Apple\nInc",
        "https://evil.test",
        "www.evil.test",
        "sec:320193",
        "CIK:abc",
    ] {
        assert!(research(text).validate(today()).is_err(), "{text}");
    }
    let json = serde_json::to_string(&research("Apple")).unwrap();
    assert!(parse_intent(&format!("```json\n{json}\n```"), today()).is_err());
    assert!(parse_intent(&format!("{json}{json}"), today()).is_err());
    assert!(
        parse_intent(
            &json.replace("\"workflow\":", "\"provider\":\"sec\",\"workflow\":"),
            today()
        )
        .is_err()
    );
    assert!(
        parse_intent(
            &json.replace("\"text\":", "\"cik\":\"320193\",\"text\":"),
            today()
        )
        .is_err()
    );
    assert!(parse_intent(&" ".repeat(8193), today()).is_err());
}

#[test]
fn enforces_workflow_cardinality_and_clarification() {
    let mut intent = research("Apple");
    intent.workflow = Workflow::Compare;
    assert!(intent.validate(today()).is_err());
    intent.subjects.push(SubjectMention {
        text: "Microsoft".into(),
        exchange: None,
    });
    assert!(intent.validate(today()).is_ok());
    intent.subjects.push(SubjectMention {
        text: "Amazon".into(),
        exchange: None,
    });
    assert!(intent.validate(today()).is_err());
    intent.subjects.clear();
    intent.workflow = Workflow::Clarify;
    assert!(intent.validate(today()).is_err());
    intent.clarification = Some("Which company?".into());
    assert!(intent.validate(today()).is_ok());
    intent.workflow = Workflow::Conversation;
    assert!(intent.validate(today()).is_err());
    intent.clarification = None;
    assert!(intent.validate(today()).is_ok());
}

#[test]
fn enforces_paired_historical_calendar_decade() {
    let mut intent = research("Apple");
    intent.start = NaiveDate::from_ymd_opt(2016, 9, 10);
    assert!(intent.validate(today()).is_err());
    intent.end = Some(today());
    assert!(intent.validate(today()).is_ok());
    intent.start = NaiveDate::from_ymd_opt(2016, 9, 9);
    assert!(intent.validate(today()).is_err());
    intent.start = Some(today());
    intent.end = NaiveDate::from_ymd_opt(2026, 9, 11);
    assert!(intent.validate(today()).is_err());
    intent.end = NaiveDate::from_ymd_opt(2026, 9, 9);
    assert!(intent.validate(today()).is_err());
}
