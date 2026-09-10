use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize};
use tokio::sync::{mpsc, watch};

use crate::error::{Error, Result};
use crate::tools::{ToolExecutor, ToolSpec, validate_tool_specs};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunSubject {
    Conversation { id: String },
    Thesis { id: String },
}

impl RunSubject {
    pub fn id(&self) -> &str {
        match self {
            Self::Conversation { id } | Self::Thesis { id } => id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunRequest {
    pub run_id: String,
    pub subject: RunSubject,
    pub instructions: String,
    pub context: String,
    pub prompt: String,
    /// Native research is an explicit host capability; missing serialized values fail closed.
    #[serde(default)]
    pub allow_web_search: bool,
    pub tools: Vec<ToolSpec>,
    pub limits: RunLimits,
}

// Presence is strict: null does not mean an absent identity field.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRequestWire {
    run_id: String,
    #[serde(default, deserialize_with = "present")]
    subject: Option<RunSubject>,
    #[serde(default, deserialize_with = "present")]
    thesis_id: Option<String>,
    instructions: String,
    context: String,
    prompt: String,
    #[serde(default)]
    allow_web_search: bool,
    tools: Vec<ToolSpec>,
    limits: RunLimits,
}

impl<'de> Deserialize<'de> for RunRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let wire = RunRequestWire::deserialize(deserializer)?;
        let subject = match (wire.subject, wire.thesis_id) {
            (Some(subject), None) => subject,
            (None, Some(id)) => RunSubject::Thesis { id },
            _ => {
                return Err(serde::de::Error::custom(
                    "exactly one run identity is required",
                ));
            }
        };
        Ok(Self {
            run_id: wire.run_id,
            subject,
            instructions: wire.instructions,
            context: wire.context,
            prompt: wire.prompt,
            allow_web_search: wire.allow_web_search,
            tools: wire.tools,
            limits: wire.limits,
        })
    }
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
        ("subject.id", request.subject.id()),
        ("prompt", request.prompt.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(Error::InvalidRequest(format!("{field} must not be empty")));
        }
    }

    if request.subject.id().len() > 256 || request.subject.id().chars().any(char::is_control) {
        return Err(Error::InvalidRequest(
            "subject.id must be bounded control-free text".into(),
        ));
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
