use crate::{AppError, ErrorKind, Result, Scope};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ConversationLimits;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conversation {
    pub id: String,
    pub workspace_id: String,
    pub repository_id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
}

impl Conversation {
    pub fn validate(&self) -> Result<()> {
        for id in [&self.id, &self.workspace_id, &self.repository_id] {
            validate_id(id)?;
        }
        if self.title.trim().is_empty()
            || self.title.len() > 1024
            || self.title.chars().any(char::is_control)
        {
            return Err(invalid(
                "conversation title must be bounded control-free text",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub role: MessageRole,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

impl Message {
    pub fn validate(&self, limits: &ConversationLimits) -> Result<()> {
        for id in [&self.id, &self.conversation_id, &self.run_id] {
            validate_id(id)?;
        }
        if self.text.trim().is_empty() {
            return Err(invalid("message text must not be empty"));
        }
        let max = match self.role {
            MessageRole::User => limits.message_bytes,
            MessageRole::Assistant => limits.assistant_bytes,
        };
        crate::agent_contract::check_serialized_size(self, max)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Admitted,
    Running,
    Completed,
    Failed,
    Interrupted,
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Interrupted)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRecord {
    /// Unverified user-supplied resolution hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company_hint: Option<String>,
    pub id: String,
    pub conversation_id: String,
    pub workspace_id: String,
    pub request_id: String,
    pub user_message_id: String,
    pub epoch: u64,
    pub status: RunStatus,
    pub input: ContextSnapshot,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<AppError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceState {
    pub conversation_id: String,
    pub workspace_id: String,
    pub revision: u64,
    pub view_ids: Vec<String>,
    pub selected_view_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SelectedReference {
    Dataset { id: String },
    View { id: String },
    Binding { id: String },
    Passage { id: String },
}

impl SelectedReference {
    pub fn id(&self) -> &str {
        match self {
            Self::Dataset { id }
            | Self::View { id }
            | Self::Binding { id }
            | Self::Passage { id } => id,
        }
    }
    pub fn validate(&self) -> Result<()> {
        validate_id(self.id())
    }
}

/// Created only from scoped trusted reads. A checksum is integrity metadata, not authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenReference {
    pub reference: SelectedReference,
    pub serialized: String,
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendMessageRequest {
    /// Unverified user-supplied resolution hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company_hint: Option<String>,
    pub conversation_id: String,
    pub request_id: String,
    pub text: String,
    pub selected: Vec<SelectedReference>,
}

impl SendMessageRequest {
    pub fn validate(&self, limits: &ConversationLimits) -> Result<()> {
        limits.validate()?;
        validate_id(&self.conversation_id)?;
        validate_id(&self.request_id)?;
        if self.text.trim().is_empty() {
            return Err(invalid("message text must not be empty"));
        }
        if self.company_hint.as_ref().is_some_and(|hint| {
            hint.trim().is_empty() || hint.len() > 256 || hint.chars().any(char::is_control)
        }) {
            return Err(invalid("company hint must be bounded control-free text"));
        }
        if self.selected.len() > limits.selected_refs {
            return Err(resource_limit());
        }
        for reference in &self.selected {
            reference.validate()?;
        }
        crate::agent_contract::check_serialized_size(self, limits.message_bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextExchange {
    /// Chronological user/assistant pair when completed; original user only otherwise.
    pub messages: Vec<Message>,
    pub status: RunStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextSnapshot {
    pub policy: String,
    pub serialized: String,
    pub message_ids: Vec<String>,
    pub references: Vec<FrozenReference>,
    pub omitted_messages: usize,
}

pub fn validate_id(id: &str) -> Result<()> {
    if id.trim().is_empty() || id.len() > Scope::MAX_ID_BYTES || id.chars().any(char::is_control) {
        Err(invalid(
            "conversation identifier must be bounded control-free text",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn invalid(message: &'static str) -> AppError {
    AppError::new(ErrorKind::InvalidInput, message, false)
}
pub(super) fn resource_limit() -> AppError {
    AppError::new(
        ErrorKind::ResourceLimit,
        "conversation data exceeds configured bounds",
        false,
    )
}
