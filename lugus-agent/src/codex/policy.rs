use serde_json::{Value, json};

use crate::{Error, Result};

/// Builds the verified 0.153.4 session override used for unattended turns.
/// The supplied names are derived in memory from `config/read`; no config
/// layer or credential-bearing response is written or logged.
pub(super) fn unattended_config(mcp_server_ids: &[String]) -> Value {
    let disabled_mcp_servers = mcp_server_ids
        .iter()
        .map(|id| (id.clone(), json!({"enabled": false})))
        .collect::<serde_json::Map<_, _>>();

    json!({
        "features": {
            "shell_tool": false,
            "hooks": false,
            "apps": false,
            "plugins": false,
            "browser_use": false,
            "computer_use": false,
            "image_generation": false,
            "view_image": false,
            "multi_agent": false,
            "goals": false,
            "sleep_tool": false,
            "tool_suggest": false,
            "skill_search": false,
            "request_permissions_tool": false
        },
        "tools": {
            "experimental_request_user_input": {"enabled": false},
            "update_plan": {"enabled": false}
        },
        "skills": {
            "include_instructions": false,
            "bundled": {"enabled": false}
        },
        "project_doc_max_bytes": 0,
        "developer_instructions": "",
        "web_search": "live",
        "mcp_servers": disabled_mcp_servers,
    })
}

/// Extracts only server identifiers from the effective config. The full
/// response can contain private configuration and must never leave this call.
pub(super) fn mcp_server_ids(config_read: &Value) -> Result<Vec<String>> {
    let config = config_read
        .get("config")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::Configuration("config/read returned no config object".into()))?;
    let Some(servers) = config.get("mcp_servers") else {
        return Ok(Vec::new());
    };
    let servers = servers
        .as_object()
        .ok_or_else(|| Error::Configuration("config/read returned malformed mcp_servers".into()))?;
    Ok(servers.keys().cloned().collect())
}
