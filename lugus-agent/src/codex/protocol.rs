use serde_json::{Value, json};

use crate::{Error, Result, ToolResult};

/// A Codex app-server envelope. This stays inside the adapter so provider
/// protocol details do not cross the runtime boundary.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WireMessage {
    Response {
        id: Value,
        outcome: ResponseOutcome,
    },
    Request {
        id: Value,
        method: RequestMethod,
        params: Value,
    },
    Notification {
        method: NotificationMethod,
        params: Option<Value>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ResponseOutcome {
    Result(Value),
    Error(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RequestMethod {
    DynamicToolCall,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NotificationMethod {
    TurnCompleted,
    Unknown(String),
}

/// Classifies a protocol frame without accepting JSON-RPC's `jsonrpc` header;
/// Codex app-server frames deliberately omit it.
pub(crate) fn classify_message(value: Value) -> Result<WireMessage> {
    let object = value.as_object().ok_or_else(malformed_envelope)?;
    let id = object.get("id");
    let method = object.get("method");
    let result = object.get("result");
    let error = object.get("error");

    match (id, method, result, error) {
        (Some(id), Some(Value::String(method)), None, None) => {
            validate_request_id(id)?;
            let params = object
                .get("params")
                .cloned()
                .ok_or_else(malformed_envelope)?;
            let request_method = match method.as_str() {
                "item/tool/call" => {
                    validate_dynamic_tool_call(&params)?;
                    RequestMethod::DynamicToolCall
                }
                _ => RequestMethod::Unknown(method.clone()),
            };
            Ok(WireMessage::Request {
                id: id.clone(),
                method: request_method,
                params,
            })
        }
        (None, Some(Value::String(method)), None, None) => Ok(WireMessage::Notification {
            method: match method.as_str() {
                "turn/completed" => NotificationMethod::TurnCompleted,
                _ => NotificationMethod::Unknown(method.clone()),
            },
            params: object.get("params").cloned(),
        }),
        (Some(id), None, Some(result), None) => {
            validate_request_id(id)?;
            Ok(WireMessage::Response {
                id: id.clone(),
                outcome: ResponseOutcome::Result(result.clone()),
            })
        }
        (Some(id), None, None, Some(error)) => {
            validate_request_id(id)?;
            Ok(WireMessage::Response {
                id: id.clone(),
                outcome: ResponseOutcome::Error(error.clone()),
            })
        }
        _ => Err(malformed_envelope()),
    }
}

/// Encodes a response to the server's `item/tool/call` request.
pub(crate) fn tool_response(id: Value, result: &ToolResult) -> Value {
    json!({
        "id": id,
        "result": {
            "contentItems": [{"type": "inputText", "text": result.content}],
            "success": result.success,
        }
    })
}

/// Produces the required response for an unsupported server request.
pub(crate) fn method_not_found_response(id: Value) -> Value {
    json!({
        "id": id,
        "error": {"code": -32601, "message": "Method not found"}
    })
}

fn validate_request_id(id: &Value) -> Result<()> {
    if id.is_string() || id.is_i64() || id.is_u64() {
        Ok(())
    } else {
        Err(malformed_envelope())
    }
}

fn validate_dynamic_tool_call(params: &Value) -> Result<()> {
    let object = params.as_object().ok_or_else(malformed_tool_call)?;
    for field in ["callId", "threadId", "tool", "turnId"] {
        if !matches!(object.get(field), Some(Value::String(_))) {
            return Err(malformed_tool_call());
        }
    }
    if !object.contains_key("arguments") {
        return Err(malformed_tool_call());
    }
    if let Some(namespace) = object.get("namespace")
        && !namespace.is_string()
        && !namespace.is_null()
    {
        return Err(malformed_tool_call());
    }
    Ok(())
}

fn malformed_envelope() -> Error {
    Error::Protocol("malformed protocol envelope".into())
}

fn malformed_tool_call() -> Error {
    Error::Protocol("malformed dynamic tool call".into())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        NotificationMethod, RequestMethod, ResponseOutcome, WireMessage, classify_message,
        method_not_found_response, tool_response,
    };
    use crate::ToolResult;

    #[test]
    fn encodes_dynamic_tool_result() {
        let result = ToolResult {
            success: true,
            content: "saved".into(),
        };
        assert_eq!(
            tool_response(json!(42), &result),
            json!({
                "id":42,"result":{"contentItems":[{"type":"inputText","text":"saved"}],
                "success":true}
            })
        );
    }

    #[test]
    fn accepts_string_server_request_ids() {
        let message = classify_message(json!({
            "id": "tool-request-1",
            "method": "item/tool/call",
            "params": {
                "arguments": {"query": "thesis"}, "callId": "call-1",
                "threadId": "thread-1", "tool": "lugus_recall", "turnId": "turn-1"
            }
        }))
        .unwrap();

        assert!(matches!(message, WireMessage::Request {
            id, method: RequestMethod::DynamicToolCall, ..
        } if id == json!("tool-request-1")));
    }

    #[test]
    fn accepts_numeric_server_request_ids() {
        let message = classify_message(json!({
            "id": 42,
            "method": "item/tool/call",
            "params": {
                "arguments": null, "callId": "call-1", "threadId": "thread-1",
                "tool": "lugus_recall", "turnId": "turn-1", "namespace": null
            }
        }))
        .unwrap();

        assert!(matches!(message, WireMessage::Request {
            id, method: RequestMethod::DynamicToolCall, ..
        } if id == json!(42)));
    }

    #[test]
    fn rejects_malformed_dynamic_tool_calls() {
        let error = classify_message(json!({
            "id": 1, "method": "item/tool/call",
            "params": {"arguments": {}, "callId": "call-1", "threadId": "thread-1", "tool": "lugus_recall"}
        }))
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "protocol error: malformed dynamic tool call"
        );
    }

    #[test]
    fn preserves_response_errors_for_the_session_to_correlate() {
        let message =
            classify_message(json!({"id": 7, "error": {"code": 1, "message": "no"}})).unwrap();

        assert!(matches!(message, WireMessage::Response {
            id, outcome: ResponseOutcome::Error(error)
        } if id == json!(7) && error["code"] == 1));
    }

    #[test]
    fn rejects_ambiguous_envelopes() {
        let error =
            classify_message(json!({"id": 1, "method": "turn/start", "result": {}})).unwrap_err();

        assert_eq!(
            error.to_string(),
            "protocol error: malformed protocol envelope"
        );
    }

    #[test]
    fn classifies_unknown_notifications_for_the_session_to_ignore() {
        let message =
            classify_message(json!({"method": "future/notice", "params": {"secret": "redacted"}}))
                .unwrap();

        assert!(matches!(message, WireMessage::Notification {
            method: NotificationMethod::Unknown(method), ..
        } if method == "future/notice"));
    }

    #[test]
    fn unknown_requests_have_a_method_not_found_response() {
        let message =
            classify_message(json!({"id": 9, "method": "future/request", "params": {}})).unwrap();
        assert!(matches!(message, WireMessage::Request {
            method: RequestMethod::Unknown(method), ..
        } if method == "future/request"));
        assert_eq!(
            method_not_found_response(json!(9)),
            json!({"id": 9, "error": {"code": -32601, "message": "Method not found"}})
        );
    }
}
