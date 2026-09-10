use lugus_agent::tools::ToolCall;
use lugus_app::{
    Catalog, ErrorKind, FetchCommand, ProviderEntry, ProviderIdentity, Scope,
    agent_contract::{decode_fetch_call, fetch_tool_specs, scope_for_call},
};
use serde_json::json;
fn entry(id: &str, capability: &str, version: u32, active: bool) -> ProviderEntry {
    ProviderEntry {
        identity: ProviderIdentity {
            instance_id: id.into(),
            plugin_id: "fixture".into(),
            plugin_version: "1".into(),
        },
        active,
        available: true,
        capabilities: [(capability.into(), version)].into(),
    }
}
fn call() -> ToolCall {
    ToolCall {
        run_id: "run-1".into(),
        call_id: "call-1".into(),
        name: "lugus_fetch_prices".into(),
        arguments: json!({"instance_id":"a","query":{"instrument":{"namespace":"fixture:symbol","value":"IBM"},"start":"2024-01-01","end":"2024-12-31","cursor":null,"page_size":10}}),
    }
}
#[test]
fn offerings_share_operation_names_and_expose_only_eligible_instances() {
    let catalog = Catalog::new(vec![
        entry("a", "market_data", 1, true),
        entry("b", "market_data", 1, true),
        entry("c", "market_data", 2, true),
        entry("d", "fundamentals", 1, false),
    ])
    .unwrap();
    let tools = fetch_tool_specs(&catalog.snapshot());
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "lugus_fetch_prices");
    assert_eq!(
        tools[0].input_schema["properties"]["instance_id"]["enum"],
        json!(["a", "b"])
    );
    assert!(fetch_tool_specs(&Catalog::new(vec![]).unwrap().snapshot()).is_empty());
}
#[test]
fn strict_decode_rejects_scope_and_operation_injection_and_bounds_input() {
    assert!(
        matches!(decode_fetch_call(&call(),4096).unwrap(),FetchCommand::Prices{instance_id,..} if instance_id=="a")
    );
    for field in ["workspace_id", "operation"] {
        let mut forged = call();
        forged.arguments[field] = json!("replacement");
        assert_eq!(
            decode_fetch_call(&forged, 4096).unwrap_err().kind,
            ErrorKind::InvalidInput
        );
    }
    let mut forged = call();
    forged.arguments["query"]["instrument"]["surprise"] = json!(true);
    assert_eq!(
        decode_fetch_call(&forged, 4096).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        decode_fetch_call(&call(), 10).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    let mut unknown = call();
    unknown.name = "provider_supplied_renderer".into();
    assert_eq!(
        decode_fetch_call(&unknown, 4096).unwrap_err().kind,
        ErrorKind::Unsupported
    );
}
#[test]
fn host_scope_rejects_forged_runs_and_keys_calls_without_concatenation_aliases() {
    let scope = Scope {
        workspace_id: "workspace".into(),
        request_id: "request".into(),
        run_id: Some("run-1".into()),
    };
    let first = scope_for_call(&scope, &call()).unwrap();
    assert_eq!(first, scope_for_call(&scope, &call()).unwrap());
    assert_eq!(first.workspace_id, "workspace");
    assert_eq!(first.run_id.as_deref(), Some("run-1"));
    assert!(first.request_id.len() <= Scope::MAX_ID_BYTES);
    let mut forged = call();
    forged.run_id = "other".into();
    assert_eq!(
        scope_for_call(&scope, &forged).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    let mut left = scope.clone();
    left.request_id = "ab".into();
    let mut right = scope;
    right.request_id = "a".into();
    let mut a = call();
    a.call_id = "c".into();
    let mut b = call();
    b.call_id = "bc".into();
    assert_ne!(
        scope_for_call(&left, &a).unwrap().request_id,
        scope_for_call(&right, &b).unwrap().request_id
    );
    let mut invalid = call();
    invalid.call_id = "".into();
    assert_eq!(
        scope_for_call(&left, &invalid).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}
