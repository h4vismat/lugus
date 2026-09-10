use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub run_id: String,
    pub call_id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub content: String,
}

#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, call: ToolCall) -> ToolResult;
}

pub(crate) fn validate_tool_specs(tools: &[ToolSpec]) -> Result<()> {
    let mut names = HashSet::with_capacity(tools.len());

    for tool in tools {
        if !is_valid_tool_name(&tool.name) {
            return Err(Error::InvalidRequest(format!(
                "invalid tool name: {}",
                tool.name
            )));
        }
        if !names.insert(tool.name.as_str()) {
            return Err(Error::InvalidRequest(format!(
                "duplicate tool name: {}",
                tool.name
            )));
        }
        if !tool.input_schema.is_object() {
            return Err(Error::InvalidRequest(format!(
                "input schema for tool {} must be an object",
                tool.name
            )));
        }
    }

    Ok(())
}

fn is_valid_tool_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}
