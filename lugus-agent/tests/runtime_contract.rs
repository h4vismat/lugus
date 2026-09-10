use lugus_agent::{RunLimits, RunRequest, ToolSpec, validate_request};

fn valid_request() -> RunRequest {
    RunRequest {
        allow_web_search: true,
        run_id: "run-1".into(),
        subject: lugus_agent::RunSubject::Thesis {
            id: "thesis-1".into(),
        },
        instructions: "Analyze evidence".into(),
        context: "No prior assessment".into(),
        prompt: "Review this thesis".into(),
        tools: vec![],
        limits: RunLimits {
            timeout: std::time::Duration::from_secs(30),
            max_tool_calls: 0,
            max_tool_result_bytes: 4096,
        },
    }
}

#[test]
fn rejects_duplicate_tool_names() {
    let tool = ToolSpec {
        name: "lugus_recall".into(),
        description: "Read stored knowledge".into(),
        input_schema: serde_json::json!({"type":"object","properties":{}}),
    };
    let request = RunRequest {
        allow_web_search: true,
        run_id: "run-1".into(),
        subject: lugus_agent::RunSubject::Thesis {
            id: "thesis-1".into(),
        },
        instructions: "Analyze evidence".into(),
        context: "No prior assessment".into(),
        prompt: "Review this thesis".into(),
        tools: vec![tool.clone(), tool],
        limits: RunLimits {
            timeout: std::time::Duration::from_secs(30),
            max_tool_calls: 8,
            max_tool_result_bytes: 4096,
        },
    };
    assert!(validate_request(&request).is_err());
}

#[test]
fn rejects_empty_run_id() {
    let request = RunRequest {
        run_id: "".into(),
        ..valid_request()
    };

    assert!(validate_request(&request).is_err());
}

#[test]
fn rejects_empty_thesis_id() {
    let request = RunRequest {
        subject: lugus_agent::RunSubject::Thesis { id: "   ".into() },
        ..valid_request()
    };

    assert!(validate_request(&request).is_err());
}

#[test]
fn rejects_empty_prompt() {
    let request = RunRequest {
        prompt: "\n\t".into(),
        ..valid_request()
    };

    assert!(validate_request(&request).is_err());
}

#[test]
fn rejects_invalid_tool_names() {
    for name in [
        String::new(),
        "contains space".into(),
        "dot.name".into(),
        "a".repeat(65),
    ] {
        let request = RunRequest {
            tools: vec![ToolSpec {
                name: name.clone(),
                description: "A tool".into(),
                input_schema: serde_json::json!({}),
            }],
            ..valid_request()
        };

        assert!(
            validate_request(&request).is_err(),
            "tool name {name:?} should be rejected"
        );
    }
}

#[test]
fn rejects_non_object_tool_schema() {
    let request = RunRequest {
        tools: vec![ToolSpec {
            name: "lugus_recall".into(),
            description: "Read stored knowledge".into(),
            input_schema: serde_json::json!([]),
        }],
        ..valid_request()
    };

    assert!(validate_request(&request).is_err());
}

#[test]
fn rejects_zero_timeout() {
    let request = RunRequest {
        limits: RunLimits {
            timeout: std::time::Duration::ZERO,
            ..valid_request().limits
        },
        ..valid_request()
    };

    assert!(validate_request(&request).is_err());
}

#[test]
fn rejects_zero_tool_result_bound() {
    let request = RunRequest {
        limits: RunLimits {
            max_tool_result_bytes: 0,
            ..valid_request().limits
        },
        ..valid_request()
    };

    assert!(validate_request(&request).is_err());
}

#[test]
fn accepts_tool_free_request_with_empty_context_and_zero_call_limit() {
    let request = RunRequest {
        context: String::new(),
        tools: vec![],
        limits: RunLimits {
            max_tool_calls: 0,
            ..valid_request().limits
        },
        ..valid_request()
    };

    assert!(validate_request(&request).is_ok());
}

#[test]
fn accepts_conversation_subject_without_a_thesis() {
    let mut wire = serde_json::to_value(valid_request()).unwrap();
    wire.as_object_mut().unwrap().remove("thesis_id");
    wire["subject"] = serde_json::json!({"kind":"conversation","id":"conversation-1"});
    let decoded: Result<RunRequest, _> = serde_json::from_value(wire.clone());
    assert!(
        decoded.is_ok(),
        "conversation identity must decode independently"
    );
    let decoded = decoded.unwrap();
    assert!(validate_request(&decoded).is_ok());
    assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
}

#[test]
fn legacy_thesis_identity_serializes_as_an_explicit_subject() {
    let mut wire = serde_json::to_value(valid_request()).unwrap();
    wire.as_object_mut().unwrap().remove("subject");
    wire["thesis_id"] = "thesis-1".into();
    let decoded: RunRequest = serde_json::from_value(wire).unwrap();
    let output = serde_json::to_value(decoded).unwrap();
    assert_eq!(
        output["subject"],
        serde_json::json!({"kind":"thesis","id":"thesis-1"})
    );
    assert!(output.get("thesis_id").is_none());
}

#[test]
fn rejects_unknown_or_conflicting_run_identity_fields() {
    for extra in [
        serde_json::json!({"subject":{"kind":"thesis","id":"thesis-1"}}),
        serde_json::json!({"typo":true}),
    ] {
        let mut wire = serde_json::to_value(valid_request()).unwrap();
        wire.as_object_mut().unwrap().remove("subject");
        wire["thesis_id"] = "thesis-1".into();
        wire.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(serde_json::from_value::<RunRequest>(wire).is_err());
    }
}

#[test]
fn subject_kind_and_fields_are_strict() {
    use lugus_agent::RunSubject;
    assert_eq!(
        serde_json::to_value(RunSubject::Conversation {
            id: "conversation-1".into()
        })
        .unwrap(),
        serde_json::json!({"kind":"conversation","id":"conversation-1"})
    );
    for subject in [
        serde_json::json!({"kind":"conversation","id":"c","thesis_id":"t"}),
        serde_json::json!({"kind":"other","id":"c"}),
        serde_json::json!({"kind":"thesis","id":"t","extra":true}),
    ] {
        assert!(serde_json::from_value::<RunSubject>(subject).is_err());
    }
    for id in ["", "  ", "bad\nidentifier"] {
        let request = RunRequest {
            subject: RunSubject::Conversation { id: id.into() },
            ..valid_request()
        };
        assert!(validate_request(&request).is_err());
    }
    let request = RunRequest {
        subject: RunSubject::Conversation {
            id: "x".repeat(257),
        },
        ..valid_request()
    };
    assert!(validate_request(&request).is_err());
}

#[test]
fn rejects_null_identity_fields_even_alongside_valid_identity() {
    for field in ["subject", "thesis_id"] {
        let mut wire = serde_json::to_value(valid_request()).unwrap();
        wire[field] = serde_json::Value::Null;
        assert!(serde_json::from_value::<RunRequest>(wire).is_err());
    }
}
