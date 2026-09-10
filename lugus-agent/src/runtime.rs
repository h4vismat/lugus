use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};

use crate::error::{Error, Result};
use crate::tools::{ToolExecutor, ToolSpec, validate_tool_specs};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRequest {
    pub run_id: String,
    pub thesis_id: String,
    pub instructions: String,
    pub context: String,
    pub prompt: String,
    pub tools: Vec<ToolSpec>,
    pub limits: RunLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLimits {
    pub timeout: Duration,
    pub max_tool_calls: usize,
    pub max_tool_result_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEvent {
    Started {
        run_id: String,
    },
    TextDelta {
        text: String,
    },
    ToolStarted {
        call_id: String,
        name: String,
    },
    ToolFinished {
        call_id: String,
        success: bool,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReport {
    pub run_id: String,
    pub outcome: RunOutcome,
    pub final_text: String,
}

#[async_trait::async_trait]
pub trait AgentRuntime: Send {
    async fn run(
        &mut self,
        request: RunRequest,
        tools: &dyn ToolExecutor,
        events: mpsc::Sender<RuntimeEvent>,
        cancel: watch::Receiver<bool>,
    ) -> Result<RunReport>;

    async fn close(&mut self) -> Result<()>;
}

pub fn validate_request(request: &RunRequest) -> Result<()> {
    for (field, value) in [
        ("run_id", request.run_id.as_str()),
        ("thesis_id", request.thesis_id.as_str()),
        ("prompt", request.prompt.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(Error::InvalidRequest(format!("{field} must not be empty")));
        }
    }

    if request.limits.timeout.is_zero() {
        return Err(Error::InvalidRequest("timeout must be positive".into()));
    }
    if request.limits.max_tool_result_bytes == 0 {
        return Err(Error::InvalidRequest(
            "max_tool_result_bytes must be positive".into(),
        ));
    }

    validate_tool_specs(&request.tools)
}
