//! Async conversation access; read handles require neither a runtime nor an execution lease.
use super::*;
use crate::conversations::*;
impl Application {
    // Authority/control values must never acquire Serialize merely to cross this boundary.
    // Store ports preflight their serializable outputs; only trusted crate code uses this bridge.
    pub(crate) async fn conversation_effect<T: Send + 'static>(
        &self,
        action: impl FnOnce(&mut dyn ConversationStore) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = self.inner.store.clone();
        tokio::task::spawn_blocking(move || action(lock(&store)?.conversation_store_mut()?))
            .await
            .map_err(|_| error(ErrorKind::Storage, "conversation storage task failed"))?
    }
    pub async fn conversation_limits(&self) -> Result<ConversationLimits> {
        self.conversation_effect(|s| Ok(s.conversation_limits().clone()))
            .await
    }
    pub async fn create_conversation(&self, request_id: &str, title: &str) -> Result<Conversation> {
        crate::conversations::validate_id(request_id)?;
        if title.trim().is_empty() || title.len() > 1024 || title.chars().any(char::is_control) {
            return Err(error(ErrorKind::InvalidInput, "invalid conversation title"));
        }
        let (request_id, title) = (request_id.to_owned(), title.to_owned());
        self.conversation_effect(move |s| s.create_conversation(&request_id, &title))
            .await
    }
    pub async fn conversation(&self, id: &str) -> Result<Conversation> {
        crate::conversations::validate_id(id)?;
        let id = id.to_owned();
        self.conversation_effect(move |s| s.conversation(&id)).await
    }
    pub async fn conversations(&self, page: PageRequest) -> Result<ConversationPage<Conversation>> {
        self.conversation_effect(move |s| s.conversations(page))
            .await
    }
    pub async fn conversation_workspace(&self, id: &str) -> Result<WorkspaceState> {
        crate::conversations::validate_id(id)?;
        let id = id.to_owned();
        self.conversation_effect(move |s| s.workspace(&id)).await
    }
    pub async fn mutate_conversation_workspace(
        &self,
        id: &str,
        revision: u64,
        mutation: WorkspaceMutation,
    ) -> Result<WorkspaceState> {
        crate::conversations::validate_id(id)?;
        let id = id.to_owned();
        self.conversation_effect(move |s| s.mutate_workspace(&id, revision, &mutation))
            .await
    }
    pub async fn conversation_messages(
        &self,
        id: &str,
        page: PageRequest,
    ) -> Result<ConversationPage<Message>> {
        crate::conversations::validate_id(id)?;
        let id = id.to_owned();
        self.conversation_effect(move |s| s.messages(&id, page))
            .await
    }
    pub async fn conversation_run(&self, id: &str, run: &str) -> Result<RunRecord> {
        crate::conversations::validate_id(id)?;
        crate::conversations::validate_id(run)?;
        let (id, run) = (id.to_owned(), run.to_owned());
        self.conversation_effect(move |s| s.run(&id, &run)).await
    }
    pub async fn conversation_runs(
        &self,
        id: &str,
        page: PageRequest,
    ) -> Result<ConversationPage<RunRecord>> {
        crate::conversations::validate_id(id)?;
        let id = id.to_owned();
        self.conversation_effect(move |s| s.runs(&id, page)).await
    }
    pub async fn conversation_activity(
        &self,
        id: &str,
        run: &str,
        page: PageRequest,
    ) -> Result<ConversationPage<ActivityRecord>> {
        crate::conversations::validate_id(id)?;
        crate::conversations::validate_id(run)?;
        let (id, run) = (id.to_owned(), run.to_owned());
        self.conversation_effect(move |s| s.activity(&id, &run, page))
            .await
    }
    pub async fn conversation_tool_records(
        &self,
        id: &str,
        run: &str,
        page: PageRequest,
    ) -> Result<ConversationPage<ToolRecord>> {
        crate::conversations::validate_id(id)?;
        crate::conversations::validate_id(run)?;
        let (id, run) = (id.to_owned(), run.to_owned());
        self.conversation_effect(move |s| s.tool_records(&id, &run, page))
            .await
    }
}
