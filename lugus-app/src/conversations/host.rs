//! Admission ownership and shared, cancellation-safe shutdown.
use super::{
    runtime::{failure, isolate},
    *,
};
use crate::{Application, ErrorKind, PageRequest, Result};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{OwnedMutexGuard, OwnedSemaphorePermit, Semaphore, watch};

type Completion = watch::Receiver<Option<Result<()>>>;
struct Registered {
    request: (String, String),
    cancel: watch::Sender<bool>,
    done: Completion,
}
struct State {
    closed: bool,
    epoch: Option<ExecutionEpoch>,
    next: u64,
    supervisors: BTreeMap<u64, Registered>,
    shutdown: Option<Completion>,
    failure: Option<crate::AppError>,
}
struct Inner {
    app: Application,
    factory: Arc<dyn RuntimeFactory>,
    limits: ConversationLimits,
    options: ConversationOptions,
    state: Mutex<State>,
    admission: Arc<tokio::sync::Mutex<()>>,
    slots: Arc<Semaphore>,
    changed: watch::Sender<()>,
}
#[derive(Clone)]
pub struct ConversationHost {
    inner: Arc<Inner>,
}
impl ConversationHost {
    /// Takes responsibility for application/provider shutdown. Ordinary reads use Application.
    pub async fn start(
        app: Application,
        factory: Arc<dyn RuntimeFactory>,
        mut options: ConversationOptions,
    ) -> Result<Self> {
        let limits = app.conversation_limits().await?;
        options.run_limits = limits.effective_run_limits(options.run_limits, app.limits())?;
        let epoch = app
            .conversation_effect(|store| {
                let lease = LocalExecutionLease::acquire(store.execution_store_key())?;
                store.activate(&lease)
            })
            .await?;
        let (changed, _) = watch::channel(());
        Ok(Self {
            inner: Arc::new(Inner {
                app,
                factory,
                options,
                slots: Arc::new(Semaphore::new(limits.active_runs)),
                limits,
                admission: Arc::new(tokio::sync::Mutex::new(())),
                changed,
                state: Mutex::new(State {
                    closed: false,
                    epoch: Some(epoch),
                    next: 0,
                    supervisors: BTreeMap::new(),
                    shutdown: None,
                    failure: None,
                }),
            }),
        })
    }
    /// Acquire ownership and recover abandoned runs without creating any runtime.
    pub async fn recover(app: Application) -> Result<Self> {
        Self::start(app, Arc::new(RecoveryOnly), ConversationOptions::default()).await
    }
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        // No user callbacks execute under this lock; poison still closes through owned cleanup.
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn application(&self) -> &Application {
        &self.inner.app
    }
    pub fn limits(&self) -> &ConversationLimits {
        &self.inner.limits
    }
    pub async fn create(&self, request_id: &str, title: &str) -> Result<Conversation> {
        self.inner.app.create_conversation(request_id, title).await
    }
    pub async fn conversation(&self, id: &str) -> Result<Conversation> {
        self.inner.app.conversation(id).await
    }
    pub async fn list(&self, page: PageRequest) -> Result<ConversationPage<Conversation>> {
        self.inner.app.conversations(page).await
    }
    pub async fn workspace(&self, id: &str) -> Result<WorkspaceState> {
        self.inner.app.conversation_workspace(id).await
    }
    pub async fn mutate_workspace(
        &self,
        id: &str,
        revision: u64,
        mutation: WorkspaceMutation,
    ) -> Result<WorkspaceState> {
        self.inner
            .app
            .mutate_conversation_workspace(id, revision, mutation)
            .await
    }
    pub async fn messages(&self, id: &str, page: PageRequest) -> Result<ConversationPage<Message>> {
        self.inner.app.conversation_messages(id, page).await
    }
    pub async fn status(&self, id: &str, run: &str) -> Result<RunRecord> {
        self.inner.app.conversation_run(id, run).await
    }
    pub async fn runs(&self, id: &str, page: PageRequest) -> Result<ConversationPage<RunRecord>> {
        self.inner.app.conversation_runs(id, page).await
    }
    pub async fn context(&self, id: &str, run: &str) -> Result<ContextSnapshot> {
        Ok(self.status(id, run).await?.input)
    }
    pub async fn activity(
        &self,
        id: &str,
        run: &str,
        page: PageRequest,
    ) -> Result<ConversationPage<ActivityRecord>> {
        self.inner.app.conversation_activity(id, run, page).await
    }
    pub async fn tool_records(
        &self,
        id: &str,
        run: &str,
        page: PageRequest,
    ) -> Result<ConversationPage<ToolRecord>> {
        self.inner
            .app
            .conversation_tool_records(id, run, page)
            .await
    }

    /// Dropping this future only drops the receipt waiter once admission is registered.
    pub async fn send(&self, request: SendMessageRequest) -> Result<RunRecord> {
        request.validate(&self.inner.limits)?;
        // Waiting for the serialization gate has no writes or capacity side effects.
        let gate = self.inner.admission.clone().lock_owned().await;
        let lookup = request.clone();
        if let Some(existing) = self
            .inner
            .app
            .conversation_effect(move |s| s.lookup_request(&lookup))
            .await?
        {
            return Ok(existing);
        }
        let (receipt_tx, mut receipt) = watch::channel(None);
        {
            let mut state = self.state();
            if state.closed {
                return Err(failure(
                    ErrorKind::Unavailable,
                    "conversation admission is closed",
                ));
            }
            let permit = self.inner.slots.clone().try_acquire_owned().map_err(|_| {
                failure(
                    ErrorKind::ResourceLimit,
                    "active conversation capacity is full",
                )
            })?;
            let epoch = state.epoch.as_ref().expect("open host owns epoch").clone();
            let key = state.next;
            state.next = key.checked_add(1).ok_or_else(|| {
                failure(ErrorKind::ResourceLimit, "supervisor identifiers exhausted")
            })?;
            let (cancel, receiver) = watch::channel(false);
            let (done, completion) = watch::channel(None);
            state.supervisors.insert(
                key,
                Registered {
                    request: (request.conversation_id.clone(), request.request_id.clone()),
                    cancel: cancel.clone(),
                    done: completion,
                },
            );
            let host = self.clone();
            tokio::spawn(async move {
                let result = host
                    .admit_owned(epoch, request, gate, permit, cancel, receiver, receipt_tx)
                    .await;
                {
                    let mut state = host.state();
                    if let Err(error) = &result {
                        state.failure.get_or_insert(error.clone());
                    }
                    state.supervisors.remove(&key);
                }
                done.send_replace(Some(result));
                host.inner.changed.send_replace(());
            });
        }
        loop {
            if let Some(result) = receipt.borrow_and_update().clone() {
                return result;
            }
            receipt.changed().await.map_err(|_| {
                failure(
                    ErrorKind::Unavailable,
                    "conversation admission supervisor stopped",
                )
            })?;
        }
    }
    #[allow(clippy::too_many_arguments)]
    async fn admit_owned(
        &self,
        epoch: ExecutionEpoch,
        request: SendMessageRequest,
        gate: OwnedMutexGuard<()>,
        _permit: OwnedSemaphorePermit,
        cancel: watch::Sender<bool>,
        receiver: watch::Receiver<bool>,
        receipt: watch::Sender<Option<Result<RunRecord>>>,
    ) -> Result<()> {
        let admission_epoch = epoch.clone();
        let admission = self
            .inner
            .app
            .conversation_effect(move |s| s.admit(&admission_epoch, &request))
            .await;
        let mut run = match admission {
            Ok(run) => run,
            Err(error) => {
                receipt.send_replace(Some(Err(error)));
                return Ok(());
            }
        };
        // Admission has committed. Keep its receipt and epoch even if the next effect fails.
        let mut unresolved = None;
        let attempt = if run.status == RunStatus::Admitted {
            let start_epoch = epoch.clone();
            let (conversation, id) = (run.conversation_id.clone(), run.id.clone());
            match self
                .inner
                .app
                .conversation_effect(move |s| s.start(&start_epoch, &conversation, &id))
                .await
            {
                Ok(attempt) => {
                    // Start changes only status. No fallible read may discard this acquired attempt.
                    run.status = RunStatus::Running;
                    Some(attempt)
                }
                Err(error) => {
                    let (conversation, id) = (run.conversation_id.clone(), run.id.clone());
                    let failed = self
                        .inner
                        .app
                        .conversation_effect(move |s| {
                            // A competing claim owns its Running/terminal result; never redispatch it.
                            if error.kind == ErrorKind::Conflict
                                && let Ok(existing) = s.run(&conversation, &id)
                                && existing.status != RunStatus::Admitted
                            {
                                return Ok(existing);
                            }
                            s.fail_admission(&epoch, &conversation, &id, &error)
                        })
                        .await;
                    match failed {
                        Ok(terminal) => run = terminal,
                        Err(error) => unresolved = Some(error),
                    }
                    None
                }
            }
        } else {
            None
        };
        drop(gate);
        receipt.send_replace(Some(Ok(run.clone())));
        if let Some(error) = unresolved {
            return Err(error);
        }
        let Some(attempt) = attempt else {
            return Ok(());
        };
        let execution = isolate(super::execution::execute(
            self.inner.app.clone(),
            self.inner.factory.clone(),
            self.inner.limits.clone(),
            self.inner.options.clone(),
            run,
            attempt.clone(),
            cancel,
            receiver,
        ))
        .await;
        let outcome = match execution {
            Ok(value) => value,
            Err(error) => RunCompletion::Failed { error }.into(),
        };
        let completion = outcome.completion;
        self.inner
            .app
            .conversation_effect(move |s| match s.finish_run(&attempt, &completion) {
                Err(error)
                    if matches!(completion, RunCompletion::Completed { .. })
                        && matches!(
                            error.kind,
                            ErrorKind::InvalidInput | ErrorKind::ResourceLimit
                        ) =>
                {
                    s.finish_run(&attempt, &RunCompletion::Failed { error })
                }
                result => result,
            })
            .await?;
        outcome.cleanup_error.map_or(Ok(()), Err)
    }
    pub async fn cancel(&self, conversation: &str, run: &str) -> Result<RunRecord> {
        let record = self.status(conversation, run).await?;
        if !record.status.is_terminal() {
            for entry in self.state().supervisors.values() {
                if entry.request.0 == record.conversation_id && entry.request.1 == record.request_id
                {
                    entry.cancel.send_replace(true);
                }
            }
        }
        Ok(record)
    }
    /// Authoritative durable state; cancelling a waiter never cancels the turn.
    pub async fn wait(&self, conversation: &str, run: &str) -> Result<RunRecord> {
        let mut changed = self.inner.changed.subscribe();
        loop {
            changed.borrow_and_update();
            let record = self.status(conversation, run).await?;
            if record.status.is_terminal() {
                return Ok(record);
            }
            if !self.state().supervisors.values().any(|entry| {
                entry.request.0 == record.conversation_id && entry.request.1 == record.request_id
            }) {
                let latest = self.status(conversation, run).await?;
                if latest.status.is_terminal() {
                    return Ok(latest);
                }
                return Err(self.state().failure.clone().unwrap_or_else(|| {
                    failure(ErrorKind::Unavailable, "run has no active supervisor")
                }));
            }
            changed
                .changed()
                .await
                .map_err(|_| failure(ErrorKind::Unavailable, "run supervision stopped"))?;
        }
    }
    pub async fn shutdown(&self) -> Result<()> {
        let mut completion = {
            let mut state = self.state();
            if let Some(completion) = &state.shutdown {
                completion.clone()
            } else {
                state.closed = true;
                let pending: Vec<_> = state
                    .supervisors
                    .values()
                    .map(|entry| {
                        entry.cancel.send_replace(true);
                        entry.done.clone()
                    })
                    .collect();
                let (done, completion) = watch::channel(None);
                state.shutdown = Some(completion.clone());
                let host = self.clone();
                tokio::spawn(async move {
                    let mut failure = None;
                    for mut item in pending {
                        loop {
                            if let Some(result) = item.borrow_and_update().clone() {
                                if let Err(error) = result {
                                    failure.get_or_insert(error);
                                }
                                break;
                            }
                            if item.changed().await.is_err() {
                                failure.get_or_insert_with(|| {
                                    runtime::failure(
                                        ErrorKind::Unavailable,
                                        "turn supervisor stopped",
                                    )
                                });
                                break;
                            }
                        }
                    }
                    if let Err(error) = host.inner.app.shutdown().await {
                        failure.get_or_insert(error);
                    }
                    let mut state = host.state();
                    if let Some(error) = state.failure.clone() {
                        failure.get_or_insert(error);
                    }
                    state.epoch.take();
                    done.send_replace(Some(failure.map_or(Ok(()), Err)));
                });
                completion
            }
        };
        loop {
            if let Some(result) = completion.borrow_and_update().clone() {
                return result;
            }
            completion
                .changed()
                .await
                .map_err(|_| failure(ErrorKind::Unavailable, "shutdown supervision stopped"))?;
        }
    }
}

struct RecoveryOnly;
#[async_trait::async_trait]
impl RuntimeFactory for RecoveryOnly {
    async fn create(&self) -> Result<Box<dyn lugus_agent::AgentRuntime>> {
        Err(failure(ErrorKind::Unavailable, "no runtime is configured"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        HostBounds, Limits, RandomIds, RepositoryFactory, SqliteApplicationStore,
        SqliteRepositoryFactory, SystemClock,
    };
    #[tokio::test]
    async fn wait_recognizes_registered_request_before_run_publication() {
        let root = tempfile::tempdir().unwrap();
        let financial = root.path().join("financial.sqlite");
        let repositories = Arc::new(SqliteRepositoryFactory::new(&financial));
        repositories.initialize().unwrap();
        let store = SqliteApplicationStore::open(
            root.path().join("application.sqlite"),
            Box::new(lugus_financial::storage::SqliteRepository::open(&financial).unwrap()),
            Limits::default(),
            Box::new(SystemClock),
            Box::new(RandomIds::new().unwrap()),
        )
        .unwrap();
        let app = Application::start(
            vec![],
            repositories,
            Box::new(store),
            Limits::default(),
            HostBounds::default(),
            Box::new(RandomIds::new().unwrap()),
        )
        .await
        .unwrap();
        let host = ConversationHost::recover(app).await.unwrap();
        let c = host.create("create", "Research").await.unwrap();
        let request = SendMessageRequest {
            conversation_id: c.id.clone(),
            request_id: "one".into(),
            text: "Research".into(),
            selected: vec![],
        };
        let (cancel, _) = watch::channel(false);
        let (done, completion) = watch::channel(None);
        let epoch = {
            let mut state = host.state();
            state.supervisors.insert(
                0,
                Registered {
                    request: (c.id.clone(), request.request_id.clone()),
                    cancel,
                    done: completion,
                },
            );
            state.epoch.as_ref().unwrap().clone()
        };
        // Hold exactly the prepublication phase: registration is present, admission is durable,
        // and the admitting supervisor has not yet claimed or returned the run to its caller.
        let admission_epoch = epoch.clone();
        let run = host
            .inner
            .app
            .conversation_effect(move |s| s.admit(&admission_epoch, &request))
            .await
            .unwrap();
        let mut waiting = Box::pin(host.wait(&c.id, &run.id));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut waiting)
                .await
                .is_err(),
            "a registered unpublished run must remain supervised"
        );
        let (conversation, id) = (c.id.clone(), run.id.clone());
        host.inner
            .app
            .conversation_effect(move |s| {
                let attempt = s.start(&epoch, &conversation, &id)?;
                s.finish_run(&attempt, &RunCompletion::Interrupted)
            })
            .await
            .unwrap();
        host.state().supervisors.remove(&0);
        done.send_replace(Some(Ok(())));
        host.inner.changed.send_replace(());
        assert_eq!(waiting.await.unwrap().status, RunStatus::Interrupted);
        host.shutdown().await.unwrap();
    }
}
