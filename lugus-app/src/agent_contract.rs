//! Pure agent-call adapters. Authorization still belongs to application dispatch.
use crate::{AppError, ErrorKind, FetchCommand, Offering, Operation, Result, Scope};
use lugus_agent::tools::{ToolCall, ToolSpec};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{self, Write},
};

const OPERATIONS: [(Operation, &str, &str); 6] = [
    (
        Operation::Resolve,
        "lugus_resolve_company",
        "Resolve company input through the selected provider; preserve ambiguity.",
    ),
    (
        Operation::Lookup,
        "lugus_lookup_company",
        "Look up an explicit company identifier through the selected provider.",
    ),
    (
        Operation::Filings,
        "lugus_fetch_filings",
        "Fetch filing metadata through the selected provider.",
    ),
    (
        Operation::Facts,
        "lugus_fetch_facts",
        "Fetch reported facts through the selected provider; no statement reconciliation is implied.",
    ),
    (
        Operation::Document,
        "lugus_fetch_document",
        "Store an original primary document through the selected provider.",
    ),
    (
        Operation::Prices,
        "lugus_fetch_prices",
        "Fetch daily market history through the selected provider using an explicit native instrument identifier.",
    ),
];

pub fn fetch_tool_specs(offering: &Offering) -> Vec<ToolSpec> {
    OPERATIONS
        .iter()
        .filter_map(|(operation, name, description)| {
            let instances: BTreeSet<_> = offering
                .operations()
                .filter(|o| o.operation == *operation)
                .map(|o| o.identity.instance_id.clone())
                .collect();
            if instances.is_empty() {
                return None;
            }
            let (field, argument) = match operation {
                Operation::Resolve => ("input", text_schema(FetchCommand::MAX_TEXT_BYTES)),
                Operation::Lookup => (
                    "request",
                    object_schema([("identifier", identifier_schema())], &["identifier"]),
                ),
                Operation::Document => ("source_url", text_schema(FetchCommand::MAX_TEXT_BYTES)),
                Operation::Filings | Operation::Facts => ("query", financial_query_schema()),
                Operation::Prices => ("query", price_query_schema()),
            };
            Some(ToolSpec {
                name: (*name).into(),
                description: (*description).into(),
                input_schema: object_schema(
                    [
                        ("instance_id", json!({"type":"string","enum":instances})),
                        (field, argument),
                    ],
                    &["instance_id", field],
                ),
            })
        })
        .collect()
}

/// Decode and validate a known fetch tool. It does not authorize a provider;
/// callers must dispatch through the bound turn offering and live catalog.
pub fn decode_fetch_call(call: &ToolCall, max_input_bytes: usize) -> Result<FetchCommand> {
    let operation = OPERATIONS
        .iter()
        .find(|(_, name, _)| *name == call.name)
        .map(|(o, _, _)| *o)
        .ok_or_else(|| AppError::new(ErrorKind::Unsupported, "unsupported research tool", false))?;
    check_serialized_size(&call.arguments, max_input_bytes)?;
    let mut arguments = call
        .arguments
        .as_object()
        .cloned()
        .ok_or_else(|| invalid("tool arguments must be an object"))?;
    if arguments.contains_key("operation") {
        return Err(invalid("operation is selected by the tool name"));
    }
    arguments.insert(
        "operation".into(),
        serde_json::to_value(operation).map_err(|_| invalid("invalid operation"))?,
    );
    let command: FetchCommand = serde_json::from_value(Value::Object(arguments))
        .map_err(|_| invalid("invalid research tool arguments"))?;
    command.validate()?;
    Ok(command)
}

/// Derive an unambiguous idempotency key without accepting scope from arguments.
pub fn scope_for_call(bound: &Scope, call: &ToolCall) -> Result<Scope> {
    bound.validate()?;
    if bound.run_id.as_deref() != Some(call.run_id.as_str()) {
        return Err(AppError::new(
            ErrorKind::ScopeMismatch,
            "tool call does not belong to the bound run",
            false,
        ));
    }
    Scope {
        workspace_id: bound.workspace_id.clone(),
        request_id: call.call_id.clone(),
        run_id: Some(call.run_id.clone()),
    }
    .validate()?;
    let encoded = serde_json::to_vec(&(
        &bound.workspace_id,
        &bound.request_id,
        &bound.run_id,
        &call.call_id,
    ))
    .map_err(|_| invalid("invalid call identity"))?;
    Ok(Scope {
        workspace_id: bound.workspace_id.clone(),
        request_id: format!("tool:{:x}", Sha256::digest(encoded)),
        run_id: bound.run_id.clone(),
    })
}

/// Count serialization through a bounded sink before cloning or encoding values.
pub fn check_serialized_size<T: serde::Serialize>(value: &T, max_bytes: usize) -> Result<()> {
    if max_bytes == 0 {
        return Err(invalid("positive input size limit required"));
    }
    struct Budget(usize);
    impl Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.0 {
                return Err(io::Error::other("size limit"));
            }
            self.0 -= bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget(max_bytes), value).map_err(|_| {
        AppError::new(
            ErrorKind::ResourceLimit,
            "serialized value exceeds size limit",
            false,
        )
    })
}
fn invalid(message: &str) -> AppError {
    AppError::new(ErrorKind::InvalidInput, message, false)
}
fn object_schema<const N: usize>(properties: [(&str, Value); N], required: &[&str]) -> Value {
    json!({"type":"object","properties":properties.into_iter().map(|(k,v)|(k.to_string(),v)).collect::<serde_json::Map<_,_>>(),"required":required,"additionalProperties":false})
}
fn text_schema(max: usize) -> Value {
    json!({"type":"string","minLength":1,"maxLength":max})
}
fn identifier_schema() -> Value {
    object_schema(
        [("namespace", text_schema(128)), ("value", text_schema(128))],
        &["namespace", "value"],
    )
}
fn financial_query_schema() -> Value {
    object_schema(
        [
            ("company", identifier_schema()),
            ("filed_from", json!({"type":"string","format":"date"})),
            ("filed_to", json!({"type":"string","format":"date"})),
            (
                "forms",
                json!({"type":"array","items":text_schema(64),"maxItems":100}),
            ),
            ("cursor", json!({"type":"null"})),
            (
                "page_size",
                json!({"type":"integer","minimum":1,"maximum":1000}),
            ),
        ],
        &["company", "filed_from", "filed_to", "page_size"],
    )
}
fn price_query_schema() -> Value {
    object_schema(
        [
            ("instrument", identifier_schema()),
            ("start", json!({"type":"string","format":"date"})),
            ("end", json!({"type":"string","format":"date"})),
            ("cursor", json!({"type":"null"})),
            (
                "page_size",
                json!({"type":"integer","minimum":1,"maximum":1000}),
            ),
        ],
        &["instrument", "start", "end", "page_size"],
    )
}
