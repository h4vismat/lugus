use serde_json::Value;

use crate::{Error, Result};

pub(super) fn response_id_at(value: &Value, path: &[&str], method: &str) -> Result<String> {
    let mut current = value;
    for segment in path {
        current = current
            .get(*segment)
            .ok_or_else(|| Error::Protocol(format!("{method} returned a malformed result")))?;
    }
    current
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::Protocol(format!("{method} returned a malformed result")))
}

pub(super) fn validate_active(
    params: &Value,
    thread_id: &str,
    turn_id: &str,
    kind: &str,
) -> Result<()> {
    if string_field(params, "threadId", kind)? != thread_id {
        return Err(Error::Protocol(format!(
            "{kind} does not belong to the active thread"
        )));
    }
    if string_field(params, "turnId", kind)? != turn_id {
        return Err(Error::Protocol(format!(
            "{kind} does not belong to the active turn"
        )));
    }
    Ok(())
}

pub(super) fn validate_completion(params: &Value, thread_id: &str, turn_id: &str) -> Result<()> {
    if string_field(params, "threadId", "turn completion")? != thread_id {
        return Err(Error::Protocol(
            "turn completion does not belong to the active thread".into(),
        ));
    }
    if params["turn"]["id"].as_str() != Some(turn_id) {
        return Err(Error::Protocol(
            "turn completion does not belong to the active turn".into(),
        ));
    }
    Ok(())
}

pub(super) fn string_field<'a>(value: &'a Value, field: &str, kind: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol(format!("malformed {kind}")))
}

pub(super) fn agent_text(item: Option<&Value>) -> Option<&str> {
    let item = item?.as_object()?;
    (item.get("type")?.as_str()? == "agentMessage")
        .then(|| item.get("text")?.as_str())
        .flatten()
}

pub(super) fn last_agent_text(items: Option<&Value>) -> Option<&str> {
    items?
        .as_array()?
        .iter()
        .rev()
        .find_map(|item| agent_text(Some(item)))
}

pub(super) fn turn_error_message(error: Option<&Value>) -> String {
    error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("Codex turn failed")
        .to_owned()
}
