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
    run: Option<(String, String)>,
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
                    run: None,
                    request: (request.conversation_id.clone(), request.request_id.clone()),
                    cancel: cancel.clone(),
                    done: completion,
                },
            );
            let host = self.clone();
            tokio::spawn(async move {
                let result = host
                    .admit_owned(
                        key, epoch, request, gate, permit, cancel, receiver, receipt_tx,
                    )
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
        key: u64,
        epoch: ExecutionEpoch,
        request: SendMessageRequest,
        gate: OwnedMutexGuard<()>,
        _permit: OwnedSemaphorePermit,
        cancel: watch::Sender<bool>,
        receiver: watch::Receiver<bool>,
        receipt: watch::Sender<Option<Result<RunRecord>>>,
    ) -> Result<()> {
        let admission = self
            .inner
            .app
            .conversation_effect(move |s| {
                let run = s.admit(&epoch, &request)?;
                if run.status != RunStatus::Admitted {
                    return Ok((run, None));
                }
                match s.start(&epoch, &run.conversation_id, &run.id) {
                    Ok(attempt) => Ok((s.run(&run.conversation_id, &run.id)?, Some(attempt))),
                    Err(error) if error.kind == ErrorKind::Conflict => {
                        Ok((s.run(&run.conversation_id, &run.id)?, None))
                    }
                    Err(error) => Err(error),
                }
            })
            .await;
        drop(gate);
        let (run, attempt) = match admission {
            Ok(value) => value,
            Err(error) => {
                receipt.send_replace(Some(Err(error)));
                return Ok(());
            }
        };
        if let Some(entry) = self.state().supervisors.get_mut(&key) {
            entry.run = Some((run.conversation_id.clone(), run.id.clone()));
        }
        receipt.send_replace(Some(Ok(run.clone())));
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
            if !self.state().supervisors.values().any(|e| {
                e.run
                    .as_ref()
                    .is_some_and(|(c, r)| c == conversation && r == run)
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
