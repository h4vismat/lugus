use lugus_agent::{RunLimits, RunRequest, ToolSpec, validate_request};

fn valid_request() -> RunRequest {
    RunRequest {
        allow_web_search: true,
        run_id: "run-1".into(),
        thesis_id: "thesis-1".into(),
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
        thesis_id: "thesis-1".into(),
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
        thesis_id: "   ".into(),
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
