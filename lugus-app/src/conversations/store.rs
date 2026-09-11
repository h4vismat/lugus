//! Host-only persistence boundary. Ownership tokens are intentionally not serializable.
use super::*;
use crate::{AppError, PageRequest, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationPage<T> {
    pub items: Vec<T>,
    pub next_offset: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunCompletion {
    Completed { text: String },
    Failed { error: AppError },
    Interrupted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityRecord {
    pub run_id: String,
    pub sequence: u64,
    pub kind: String,
    pub data: String,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolIntent {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    pub result_capacity: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolOutcome {
    Returned { result: lugus_agent::ToolResult },
    Failed { error: AppError },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolRecord {
    pub run_id: String,
    pub request_id: String,
    pub workspace_id: String,
    pub intent: ToolIntent,
    pub outcome: Option<ToolOutcome>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginTool {
    Dispatch(ToolRecord),
    Recorded(ToolRecord),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkspaceMutation {
    Select { view_id: String },
    Reorder { view_ids: Vec<String> },
    Close { view_id: String },
}
pub trait ConversationStore: Send {
    fn conversation_limits(&self) -> &ConversationLimits;
    fn execution_store_key(&self) -> &Path;
    fn create_conversation(&mut self, request_id: &str, title: &str) -> Result<Conversation>;
    fn conversation(&self, id: &str) -> Result<Conversation>;
    fn conversations(&self, page: PageRequest) -> Result<ConversationPage<Conversation>>;
    /// Optional newest-activity listing; existing adapters retain their original contract.
    fn recent_conversations(&self, _page: PageRequest) -> Result<ConversationPage<Conversation>> {
        Err(AppError::new(
            crate::ErrorKind::Unsupported,
            "recent conversations are unavailable",
            false,
        ))
    }
    fn workspace(&self, conversation_id: &str) -> Result<WorkspaceState>;
    fn mutate_workspace(
        &mut self,
        conversation_id: &str,
        expected_revision: u64,
        mutation: &WorkspaceMutation,
    ) -> Result<WorkspaceState>;
    fn messages(
        &self,
        conversation_id: &str,
        page: PageRequest,
    ) -> Result<ConversationPage<Message>>;
    fn run(&self, conversation_id: &str, run_id: &str) -> Result<RunRecord>;
    /// Persist the exact research package once for an active, fenced attempt.
    fn save_preparation(&mut self, _attempt: &RunAttempt, _serialized: &str) -> Result<()> {
        Err(AppError::new(
            crate::ErrorKind::Unsupported,
            "research preparation is unavailable",
            false,
        ))
    }
    fn preparation(&self, _conversation_id: &str, _run_id: &str) -> Result<Option<String>> {
        Err(AppError::new(
            crate::ErrorKind::Unsupported,
            "research preparation is unavailable",
            false,
        ))
    }
    fn latest_preparation(&self, _conversation_id: &str) -> Result<Option<String>> {
        Err(AppError::new(
            crate::ErrorKind::Unsupported,
            "research preparation is unavailable",
            false,
        ))
    }
    fn runs(&self, conversation_id: &str, page: PageRequest)
    -> Result<ConversationPage<RunRecord>>;
    fn activity(
        &self,
        conversation_id: &str,
        run_id: &str,
        page: PageRequest,
    ) -> Result<ConversationPage<ActivityRecord>>;
    fn tool_records(
        &self,
        conversation_id: &str,
        run_id: &str,
        page: PageRequest,
    ) -> Result<ConversationPage<ToolRecord>>;
    /// One-shot activation recovers abandoned work; opening a store never does.
    fn activate(&mut self, lease: &LocalExecutionLease) -> Result<ExecutionEpoch>;
    fn lookup_request(&self, request: &SendMessageRequest) -> Result<Option<RunRecord>>;
    fn admit(&mut self, epoch: &ExecutionEpoch, request: &SendMessageRequest) -> Result<RunRecord>;
    /// Atomically claim Admitted -> Running, changing only status; return authority after commit.
    fn start(
        &mut self,
        epoch: &ExecutionEpoch,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<RunAttempt>;
    /// Terminalize an accepted admission that never acquired a runtime attempt.
    /// Only Admitted may transition; matching Failed retries are idempotent.
    fn fail_admission(
        &mut self,
        epoch: &ExecutionEpoch,
        conversation_id: &str,
        run_id: &str,
        error: &AppError,
    ) -> Result<RunRecord>;
    fn append_activity(
        &mut self,
        attempt: &RunAttempt,
        kind: &str,
        data: &str,
    ) -> Result<ActivityRecord>;
    fn begin_tool(&mut self, attempt: &RunAttempt, intent: &ToolIntent) -> Result<BeginTool>;
    fn finish_tool(
        &mut self,
        attempt: &RunAttempt,
        call_id: &str,
        outcome: &ToolOutcome,
    ) -> Result<ToolRecord>;
    fn finish_run(&mut self, attempt: &RunAttempt, completion: &RunCompletion)
    -> Result<RunRecord>;
}
