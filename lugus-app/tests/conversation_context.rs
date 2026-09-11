use chrono::{TimeZone, Utc};
use lugus_app::conversations::*;
use lugus_app::{ErrorKind, Limits};

fn make_message(id: &str, run: &str, role: MessageRole, text: &str) -> Message {
    Message {
        id: id.into(),
        conversation_id: "conversation-1".into(),
        run_id: run.into(),
        role,
        text: text.into(),
        created_at: Utc.timestamp_opt(0, 0).unwrap(),
    }
}
fn new_message() -> Message {
    make_message("new", "run-new", MessageRole::User, "follow up")
}
fn exchange(prefix: &str, text: &str) -> ContextExchange {
    ContextExchange {
        messages: vec![
            make_message(&format!("{prefix}-u"), prefix, MessageRole::User, text),
            make_message(
                &format!("{prefix}-a"),
                prefix,
                MessageRole::Assistant,
                "answer",
            ),
        ],
        status: RunStatus::Completed,
    }
}

#[test]
fn full_serialized_context_has_exact_byte_boundary() {
    let message = make_message("new", "run-new", MessageRole::User, "\"\\\n日本語");
    let mut limits = ConversationLimits::default();
    let snapshot = build_context(&message, &[], &[], &limits).unwrap();
    assert_eq!(snapshot.policy, "conversation-context-v1");
    assert_eq!(snapshot.message_ids, ["new"]);
    assert_eq!(snapshot.omitted_messages, 0);
    let decoded: serde_json::Value = serde_json::from_str(&snapshot.serialized).unwrap();
    assert_eq!(decoded["new_message"]["text"], "\"\\\n日本語");
    limits.context_bytes = snapshot.serialized.len();
    assert!(build_context(&message, &[], &[], &limits).is_ok());
    limits.context_bytes -= 1;
    assert_eq!(
        build_context(&message, &[], &[], &limits).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
}

#[test]
fn full_message_envelope_is_bounded_before_context() {
    let message = make_message("new", "run-new", MessageRole::User, "\"\\\n日本語");
    let mut limits = ConversationLimits {
        message_bytes: serde_json::to_vec(&message).unwrap().len(),
        ..ConversationLimits::default()
    };
    assert!(build_context(&message, &[], &[], &limits).is_ok());
    limits.message_bytes -= 1;
    assert!(build_context(&message, &[], &[], &limits).is_err());
    assert!(
        build_context(
            &make_message("oversize", "r", MessageRole::User, &"x".repeat(20_000)),
            &[],
            &[],
            &ConversationLimits::default()
        )
        .is_err()
    );
}

#[test]
fn history_is_a_chronological_suffix_of_whole_exchanges() {
    let history = vec![
        exchange("recent", "recent question"),
        exchange("large", &"x".repeat(8_000)),
        exchange("old", "small"),
    ];
    let limits = ConversationLimits {
        context_bytes: 2500,
        ..ConversationLimits::default()
    };
    let snapshot = build_context_with_omitted(&new_message(), &history, &[], 10, &limits).unwrap();
    assert_eq!(snapshot.message_ids, ["recent-u", "recent-a", "new"]);
    assert_eq!(snapshot.omitted_messages, 14);
    let data: serde_json::Value = serde_json::from_str(&snapshot.serialized).unwrap();
    assert_eq!(data["omitted_messages"], 14);
    let full = build_context(
        &new_message(),
        &[exchange("recent", "r"), exchange("old", "o")],
        &[],
        &ConversationLimits::default(),
    )
    .unwrap();
    assert_eq!(
        full.message_ids,
        ["old-u", "old-a", "recent-u", "recent-a", "new"]
    );
    assert_eq!(
        full,
        build_context(
            &new_message(),
            &[exchange("recent", "r"), exchange("old", "o")],
            &[],
            &ConversationLimits::default()
        )
        .unwrap()
    );
}

#[test]
fn message_count_never_splits_a_completed_pair() {
    let limits = ConversationLimits {
        context_messages: 2,
        ..ConversationLimits::default()
    };
    let snapshot = build_context(&new_message(), &[exchange("old", "q")], &[], &limits).unwrap();
    assert_eq!(snapshot.message_ids, ["new"]);
    assert_eq!(snapshot.omitted_messages, 2);
}

#[test]
fn interrupted_exchange_has_explicit_status_and_no_partial_assistant() {
    let interrupted = ContextExchange {
        messages: vec![make_message(
            "prior",
            "prior-run",
            MessageRole::User,
            "original",
        )],
        status: RunStatus::Interrupted,
    };
    let snapshot = build_context(
        &new_message(),
        std::slice::from_ref(&interrupted),
        &[],
        &ConversationLimits::default(),
    )
    .unwrap();
    let data: serde_json::Value = serde_json::from_str(&snapshot.serialized).unwrap();
    assert_eq!(data["exchanges"][0]["status"], "interrupted");
    assert_eq!(
        data["exchanges"][0]["messages"].as_array().unwrap().len(),
        1
    );
    let mut invalid = interrupted;
    invalid.messages.push(make_message(
        "partial",
        "prior-run",
        MessageRole::Assistant,
        "unfinished",
    ));
    assert!(
        build_context(
            &new_message(),
            &[invalid],
            &[],
            &ConversationLimits::default()
        )
        .is_err()
    );
}

#[test]
fn every_limit_rejects_zero_and_excessive_values() {
    let limits = ConversationLimits::default();
    assert!(limits.validate().is_ok());
    let wire = serde_json::to_value(&limits).unwrap();
    for key in wire.as_object().unwrap().keys() {
        for value in [0u64, u64::MAX] {
            let mut invalid = wire.clone();
            invalid[key] = value.into();
            let decoded = serde_json::from_value::<ConversationLimits>(invalid);
            assert!(
                decoded.is_err() || decoded.unwrap().validate().is_err(),
                "{key}={value}"
            );
        }
    }
    let defaults: ConversationLimits = serde_json::from_str("{}").unwrap();
    assert_eq!(defaults, limits);
    assert!(serde_json::from_str::<ConversationLimits>("{\"unknown\":1}").is_err());
    let mut invalid = limits.clone();
    invalid.runtime_close_timeout_ms = 60_001;
    assert!(invalid.validate().is_err());
    invalid.runtime_close_timeout_ms = 60_000;
    assert!(invalid.validate().is_ok());
}

#[test]
fn run_limits_are_finite_and_intersect_application_budgets() {
    let limits = ConversationLimits {
        tool_calls: 7,
        tool_record_bytes: 2048,
        tool_total_bytes: 1024,
        ..ConversationLimits::default()
    };
    let app = Limits {
        max_output_bytes: 4096,
        max_read_page_bytes: 4096,
        ..Limits::default()
    };
    let requested = default_conversation_run_limits();
    let actual = limits.effective_run_limits(requested, &app).unwrap();
    assert_eq!(actual.max_tool_calls, 7);
    assert_eq!(actual.max_tool_result_bytes, 1024);
    assert!(
        limits
            .effective_run_limits(
                lugus_agent::RunLimits {
                    timeout: std::time::Duration::from_secs(86_401),
                    ..requested
                },
                &app
            )
            .is_err()
    );
}

#[test]
fn selected_reference_request_rejects_untrusted_payload_and_bad_ids() {
    assert!(
        serde_json::from_value::<SelectedReference>(
            serde_json::json!({"kind":"dataset","id":"d","payload":{}})
        )
        .is_err()
    );
    let limits = ConversationLimits::default();
    let request = SendMessageRequest {
        company_hint: None,
        conversation_id: "c".into(),
        request_id: "req".into(),
        text: "question".into(),
        selected: vec![SelectedReference::Dataset { id: "d".into() }],
    };
    assert!(request.validate(&limits).is_ok());
    for id in ["", "bad\nid"] {
        let invalid = SendMessageRequest {
            company_hint: None,
            conversation_id: id.into(),
            ..request.clone()
        };
        assert!(invalid.validate(&limits).is_err());
    }
}

#[test]
fn frozen_view_pins_immutable_descriptor_and_required_evidence_is_not_truncated() {
    let mut view = lugus_app::ViewReceipt {
        id: "v".into(),
        workspace_id: "w".into(),
        request_id: "req".into(),
        dataset_id: "dataset-original".into(),
        kind: lugus_app::ViewKind::DataTable,
        descriptor_revision: 3,
        accepted_at: Utc.timestamp_opt(0, 0).unwrap(),
        presentation: None,
    };
    let limits = ConversationLimits::default();
    let frozen = FrozenReference::from_view(&view, &limits).unwrap();
    view.presentation = Some(lugus_app::PresentationStatus::Failed);
    assert_eq!(frozen, FrozenReference::from_view(&view, &limits).unwrap());
    let payload: FrozenView = serde_json::from_str(&frozen.serialized).unwrap();
    assert_eq!(payload.dataset_id, "dataset-original");
    assert_eq!(payload.descriptor_revision, 3);
    let snapshot =
        build_context(&new_message(), &[], std::slice::from_ref(&frozen), &limits).unwrap();
    let mut exact = limits.clone();
    exact.context_bytes = snapshot.serialized.len();
    assert!(build_context(&new_message(), &[], std::slice::from_ref(&frozen), &exact).is_ok());
    exact.context_bytes -= 1;
    assert!(build_context(&new_message(), &[], std::slice::from_ref(&frozen), &exact).is_err());
    let mut tampered = frozen;
    tampered.serialized.push(' ');
    assert!(build_context(&new_message(), &[], &[tampered], &limits).is_err());
}

#[test]
fn frozen_dataset_records_first_page_coverage_and_bounds_envelopes() {
    let page: lugus_app::DatasetPage = serde_json::from_value(serde_json::json!({
        "header":{"id":"dataset-1","workspace_id":"workspace-1","repository_id":"repo-1","provider":{"instance_id":"fixture","plugin_id":"fixture","plugin_version":"1"},"fetch_id":"fetch-1","kind":"filings","projection":{"kind":"filings","run_id":1},"query":{},"created_at":"1970-01-01T00:00:00Z","row_count":3,"limitations":[],"conflicts":[]},
        "rows":[],"next_offset":0
    })).unwrap();
    let limits = ConversationLimits::default();
    let frozen = FrozenReference::from_dataset(&page, &limits).unwrap();
    let payload: FrozenDataset = serde_json::from_str(&frozen.serialized).unwrap();
    assert_eq!(payload.coverage.offset, 0);
    assert_eq!(payload.coverage.returned_rows, 0);
    assert_eq!(payload.coverage.total_rows, 3);
    assert!(!payload.coverage.complete);
    assert_eq!(payload.coverage.next_offset, Some(0));
    let mut small = limits.clone();
    small.selected_bytes = serde_json::to_vec(&vec![frozen.clone()]).unwrap().len();
    assert!(build_context(&new_message(), &[], std::slice::from_ref(&frozen), &small).is_ok());
    small.selected_bytes -= 1;
    assert!(build_context(&new_message(), &[], &[frozen], &small).is_err());
    let mut invalid = page.clone();
    invalid.next_offset = Some(2);
    assert!(FrozenReference::from_dataset(&invalid, &limits).is_err());
}

#[test]
fn frozen_reference_rejects_relabelling_exact_payload() {
    let view = lugus_app::ViewReceipt {
        id: "view-original".into(),
        workspace_id: "w".into(),
        request_id: "req".into(),
        dataset_id: "dataset-original".into(),
        kind: lugus_app::ViewKind::DataTable,
        descriptor_revision: 1,
        accepted_at: Utc.timestamp_opt(0, 0).unwrap(),
        presentation: None,
    };
    let limits = ConversationLimits::default();
    let mut frozen = FrozenReference::from_view(&view, &limits).unwrap();
    frozen.reference = SelectedReference::View {
        id: "different-view".into(),
    };
    assert!(build_context(&new_message(), &[], &[frozen], &limits).is_err());
}

#[test]
fn context_rejects_duplicate_message_identity_across_exchanges() {
    let newest = exchange("recent", "q");
    let mut earlier = exchange("old", "q");
    earlier.messages[1].id = newest.messages[1].id.clone();
    assert!(
        build_context(
            &new_message(),
            &[newest, earlier],
            &[],
            &ConversationLimits::default()
        )
        .is_err()
    );
}

fn selected_page() -> lugus_app::DatasetPage {
    serde_json::from_value(serde_json::json!({
        "header":{"id":"dataset-1","workspace_id":"workspace-1","repository_id":"repo-1","provider":{"instance_id":"fixture","plugin_id":"fixture","plugin_version":"1"},"fetch_id":"fetch-1","kind":"filings","projection":{"kind":"filings","run_id":1},"query":{},"created_at":"1970-01-01T00:00:00Z","row_count":3,"limitations":[],"conflicts":[]},
        "rows":[],"next_offset":0
    })).unwrap()
}

fn selected_binding() -> lugus_app::BindingRecord {
    let provider =
        serde_json::json!({"instance_id":"fixture","plugin_id":"fixture","plugin_version":"1"});
    let instrument = serde_json::json!({"namespace":"fixture:symbol","value":"AAPL"});
    serde_json::from_value(serde_json::json!({
        "id":"binding-1","scope":{"workspace_id":"workspace-1","request_id":"req-1","run_id":"run-1"},"repository_id":"repo-1","company_dataset_id":"company-1",
        "company":{"company":1,"candidate":{"identifier":{"namespace":"cik","value":"320193"},"name":"Apple","aliases":[],"listings":[],"source_url":"https://example.test/company","source_checksum":"abc","retrieved_at":"1970-01-01T00:00:00Z","match_reasons":[]},"provider":provider,"run_id":1,"observation_id":1,"recorded_at":"1970-01-01T00:00:00Z"},
        "listing":{"ticker":instrument,"exchange":null},"instrument_fetch_id":"fetch-1",
        "instrument":{"id":1,"provider":provider,"request":{"instrument":instrument},"metadata":{"instrument":instrument,"issuer_name":"Apple","ticker":"AAPL","exchange":null,"kind":"equity","issuer_identifiers":[],"source_url":"https://example.test/instrument","source_checksum":"def","retrieved_at":"1970-01-01T00:00:00Z"},"recorded_at":"1970-01-01T00:00:00Z"},
        "policy":"instrument-binding-v1","reasons":[],"supersedes":null,"created_at":"1970-01-01T00:00:00Z"
    })).unwrap()
}

fn replace_frozen_payload(frozen: &mut FrozenReference, payload: serde_json::Value) {
    use sha2::{Digest, Sha256};
    frozen.serialized = serde_json::to_string(&payload).unwrap();
    frozen.checksum = format!("{:x}", Sha256::digest(frozen.serialized.as_bytes()));
}

#[test]
fn restored_dataset_obeys_current_page_limits() {
    let limits = ConversationLimits::default();
    let page = selected_page();
    let frozen = FrozenReference::from_dataset(&page, &limits).unwrap();
    let smaller = ConversationLimits {
        page_bytes: 1,
        ..limits.clone()
    };
    assert!(FrozenReference::from_dataset(&page, &smaller).is_err());
    assert!(frozen.validate(&smaller).is_err());
    let binding = selected_binding();
    let page = lugus_app::DatasetPage {
        rows: vec![
            lugus_app::DatasetRow::Candidate {
                entry: binding.company.clone(),
            },
            lugus_app::DatasetRow::Candidate {
                entry: binding.company,
            },
        ],
        next_offset: Some(2),
        ..page
    };
    let frozen = FrozenReference::from_dataset(&page, &limits).unwrap();
    let smaller = ConversationLimits {
        page_items: 1,
        ..limits.clone()
    };
    assert!(FrozenReference::from_dataset(&page, &smaller).is_err());
    assert!(frozen.validate(&smaller).is_err());
    let invalid = ConversationLimits {
        runtime_close_timeout_ms: 0,
        ..limits
    };
    assert!(frozen.validate(&invalid).is_err());
}

#[test]
fn restored_dataset_rejects_inconsistent_first_page_and_coverage() {
    let limits = ConversationLimits::default();
    let original = FrozenReference::from_dataset(&selected_page(), &limits).unwrap();
    for (pointer, replacement) in [
        ("/coverage/offset", serde_json::json!(10)),
        ("/coverage/returned_rows", serde_json::json!(1)),
        ("/coverage/total_rows", serde_json::json!(4)),
        ("/coverage/next_offset", serde_json::json!(null)),
        ("/coverage/complete", serde_json::json!(true)),
        ("/page/next_offset", serde_json::json!(2)),
        (
            "/page/header/workspace_id",
            serde_json::json!("bad\nworkspace"),
        ),
    ] {
        let mut frozen = original.clone();
        let mut payload: serde_json::Value = serde_json::from_str(&frozen.serialized).unwrap();
        *payload.pointer_mut(pointer).unwrap() = replacement;
        replace_frozen_payload(&mut frozen, payload);
        assert!(
            frozen.validate(&limits).is_err(),
            "accepted invalid {pointer}"
        );
    }
}

#[test]
fn restored_dataset_rejects_unknown_nested_fields_without_changing_legacy_dtos() {
    let limits = ConversationLimits::default();
    let page = selected_page();
    let original = FrozenReference::from_dataset(&page, &limits).unwrap();
    for pointer in ["/page", "/page/header", "/page/header/provider"] {
        let mut frozen = original.clone();
        let mut payload: serde_json::Value = serde_json::from_str(&frozen.serialized).unwrap();
        payload.pointer_mut(pointer).unwrap()["mutable_status"] = "latest".into();
        // Compatibility stays permissive for callers of the preexisting DTOs.
        assert!(serde_json::from_value::<lugus_app::DatasetPage>(payload["page"].clone()).is_ok());
        replace_frozen_payload(&mut frozen, payload);
        assert!(
            frozen.validate(&limits).is_err(),
            "accepted extra field at {pointer}"
        );
    }
}

#[test]
fn restored_binding_rechecks_scope_and_rejects_unknown_nested_fields() {
    let limits = ConversationLimits::default();
    let binding = selected_binding();
    let original = FrozenReference::from_binding(&binding, &limits).unwrap();
    assert!(original.validate(&limits).is_ok());
    for pointer in ["", "/company", "/company/provider", "/listing/ticker"] {
        let mut frozen = original.clone();
        let mut payload: serde_json::Value = serde_json::from_str(&frozen.serialized).unwrap();
        payload.pointer_mut(pointer).unwrap()["mutable_status"] = "active".into();
        assert!(serde_json::from_value::<lugus_app::BindingRecord>(payload.clone()).is_ok());
        replace_frozen_payload(&mut frozen, payload);
        assert!(
            frozen.validate(&limits).is_err(),
            "accepted extra field at {pointer}"
        );
    }
    for pointer in ["/repository_id", "/scope/workspace_id", "/scope/request_id"] {
        let mut frozen = original.clone();
        let mut payload: serde_json::Value = serde_json::from_str(&frozen.serialized).unwrap();
        *payload.pointer_mut(pointer).unwrap() = "bad\nid".into();
        replace_frozen_payload(&mut frozen, payload);
        assert!(
            frozen.validate(&limits).is_err(),
            "accepted invalid {pointer}"
        );
    }
}

#[test]
fn restored_view_rechecks_bounded_descriptor_ids() {
    let limits = ConversationLimits::default();
    let view = lugus_app::ViewReceipt {
        id: "v".into(),
        workspace_id: "w".into(),
        request_id: "req".into(),
        dataset_id: "d".into(),
        kind: lugus_app::ViewKind::DataTable,
        descriptor_revision: 1,
        accepted_at: Utc.timestamp_opt(0, 0).unwrap(),
        presentation: None,
    };
    let original = FrozenReference::from_view(&view, &limits).unwrap();
    for (field, value) in [
        ("workspace_id", "bad\nid".to_string()),
        ("dataset_id", "x".repeat(257)),
    ] {
        let mut frozen = original.clone();
        let mut payload: serde_json::Value = serde_json::from_str(&frozen.serialized).unwrap();
        payload[field] = value.into();
        replace_frozen_payload(&mut frozen, payload);
        assert!(
            frozen.validate(&limits).is_err(),
            "accepted invalid {field}"
        );
    }
}

#[test]
fn frozen_shape_check_preserves_opaque_data_and_json_formatting() {
    use sha2::{Digest, Sha256};
    let limits = ConversationLimits::default();
    let mut page = selected_page();
    page.header.query = serde_json::json!({"provider_specific":{"mutable_status":"source data","extra":[1,"日本語\n"]}});
    let mut frozen = FrozenReference::from_dataset(&page, &limits).unwrap();
    let payload: serde_json::Value = serde_json::from_str(&frozen.serialized).unwrap();
    frozen.serialized = serde_json::to_string_pretty(&payload).unwrap();
    frozen.checksum = format!("{:x}", Sha256::digest(frozen.serialized.as_bytes()));
    assert!(frozen.validate(&limits).is_ok());
    let snapshot =
        build_context(&new_message(), &[], std::slice::from_ref(&frozen), &limits).unwrap();
    assert_eq!(snapshot.references[0].serialized, frozen.serialized);
}

#[test]
fn optional_company_hint_preserves_legacy_wire_shape_and_rejects_invalid_text() {
    let legacy = r#"{"conversation_id":"c","request_id":"r","text":"research","selected":[]}"#;
    let request: SendMessageRequest = serde_json::from_str(legacy).unwrap();
    assert_eq!(request.company_hint, None);
    assert_eq!(serde_json::to_string(&request).unwrap(), legacy);
    let limits = ConversationLimits::default();
    for hint in [
        "".to_owned(),
        "  ".to_owned(),
        "AAPL\n".to_owned(),
        "é".repeat(129),
    ] {
        let invalid = SendMessageRequest {
            company_hint: Some(hint),
            ..request.clone()
        };
        assert_eq!(
            invalid.validate(&limits).unwrap_err().kind,
            ErrorKind::InvalidInput
        );
    }
    let hinted = SendMessageRequest {
        company_hint: Some("é".repeat(128)),
        ..request
    };
    hinted.validate(&limits).unwrap();
    let encoded = serde_json::to_string(&hinted).unwrap();
    assert_eq!(
        serde_json::from_str::<SendMessageRequest>(&encoded).unwrap(),
        hinted
    );
}
