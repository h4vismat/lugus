//! Shared admission, lifecycle and supervised jobs. SQL always crosses a blocking boundary.
use crate::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{broadcast, oneshot, watch};

/// A fresh 256-bit OS-random namespace plus a non-wrapping monotonic counter.
/// Construction fails closed if the OS cannot provide entropy.
pub struct RandomIds {
    prefix: String,
    counter: AtomicU64,
}
impl RandomIds {
    pub fn new() -> Result<Self> {
        let mut entropy = [0u8; 32];
        getrandom::fill(&mut entropy)
            .map_err(|_| error(ErrorKind::Unavailable, "identifier entropy is unavailable"))?;
        let prefix = entropy.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self {
            prefix,
            counter: AtomicU64::new(0),
        })
    }
}
impl IdSource for RandomIds {
    fn next_id(&self) -> String {
        let counter = self
            .counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("identifier namespace exhausted");
        format!("{}:{counter:x}", self.prefix)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostBounds {
    pub max_pending_jobs: usize,
    pub max_terminal_jobs: usize,
    pub event_capacity: usize,
}
impl Default for HostBounds {
    fn default() -> Self {
        Self {
            max_pending_jobs: 256,
            max_terminal_jobs: 256,
            event_capacity: 256,
        }
    }
}
impl HostBounds {
    pub fn validate(&self) -> Result<()> {
        if [
            self.max_pending_jobs,
            self.max_terminal_jobs,
            self.event_capacity,
        ]
        .iter()
        .all(|n| (1..=100_000).contains(n))
        {
            Ok(())
        } else {
            Err(error(ErrorKind::InvalidInput, "invalid host capacity"))
        }
    }
}
pub struct ConfiguredProvider {
    pub active: bool,
    pub factory: Arc<dyn ProviderFactory>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobReceipt {
    pub id: String,
    pub scope: Scope,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStatus {
    pub receipt: JobReceipt,
    pub state: JobState,
    pub fetch_id: Option<String>,
    pub error: Option<AppError>,
}
impl JobStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            JobState::Succeeded | JobState::Failed | JobState::Cancelled
        )
    }
}
/// Lossy notifications only; `status`/`wait` are authoritative.
#[derive(Debug, Clone, Serialize)]
pub struct ApplicationEvent {
    pub job: JobStatus,
}
struct RegisteredJob {
    instance_id: String,
    cancellation: Cancellation,
    status: watch::Sender<JobStatus>,
}
struct Admission {
    closed: bool,
    workers: BTreeMap<String, WorkerHandle>,
    jobs: BTreeMap<String, RegisteredJob>,
    terminals: VecDeque<String>,
    shutdown: Option<watch::Receiver<Option<Result<()>>>>,
}
struct ProviderControl {
    factory: Arc<dyn ProviderFactory>,
    lifecycle: Arc<tokio::sync::Mutex<()>>,
}
struct Inner {
    catalog: Arc<Mutex<Catalog>>,
    admission: Mutex<Admission>,
    providers: BTreeMap<String, ProviderControl>,
    repositories: Arc<dyn RepositoryFactory>,
    store: Arc<Mutex<Box<dyn ApplicationStore>>>,
    ids: Mutex<Box<dyn IdSource>>,
    limits: Limits,
    bounds: HostBounds,
    slots: WorkerSlots,
    events: broadcast::Sender<ApplicationEvent>,
}
#[derive(Clone)]
pub struct Application {
    inner: Arc<Inner>,
}
fn error(kind: ErrorKind, message: &'static str) -> AppError {
    AppError::new(kind, message, false)
}
fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| error(ErrorKind::Unavailable, "application state is unavailable"))
}
impl Application {
    /// Invalid configuration is rejected before repository or provider effects. Startup failures
    /// remain configured/unavailable; independent instances start concurrently.
    pub async fn start(
        providers: Vec<ConfiguredProvider>,
        repositories: Arc<dyn RepositoryFactory>,
        store: Box<dyn ApplicationStore>,
        limits: Limits,
        bounds: HostBounds,
        ids: Box<dyn IdSource>,
    ) -> Result<Self> {
        limits.validate()?;
        bounds.validate()?;
        let catalog = Catalog::new(
            providers
                .iter()
                .map(|p| ProviderEntry {
                    identity: p.factory.identity(),
                    active: p.active,
                    available: false,
                    capabilities: BTreeMap::new(),
                })
                .collect(),
        )?;
        let active: Vec<_> = providers
            .iter()
            .filter(|p| p.active)
            .map(|p| p.factory.identity().instance_id)
            .collect();
        let repo = repositories.clone();
        tokio::task::spawn_blocking(move || repo.initialize())
            .await
            .map_err(|_| error(ErrorKind::Storage, "repository initialization failed"))?
            .map_err(AppError::from)?;
        let slots = WorkerSlots::new(limits.max_concurrent_jobs)?;
        let (events, _) = broadcast::channel(bounds.event_capacity);
        let app = Self {
            inner: Arc::new(Inner {
                catalog: Arc::new(Mutex::new(catalog)),
                admission: Mutex::new(Admission {
                    closed: false,
                    workers: BTreeMap::new(),
                    jobs: BTreeMap::new(),
                    terminals: VecDeque::new(),
                    shutdown: None,
                }),
                providers: providers
                    .into_iter()
                    .map(|p| {
                        (
                            p.factory.identity().instance_id,
                            ProviderControl {
                                factory: p.factory,
                                lifecycle: Arc::new(tokio::sync::Mutex::new(())),
                            },
                        )
                    })
                    .collect(),
                repositories,
                store: Arc::new(Mutex::new(store)),
                ids: Mutex::new(ids),
                limits,
                bounds,
                slots,
                events,
            }),
        };
        // Construction may itself be dropped. Keep startup and orphan cleanup owned until
        // either the caller receives the host or every admitted provider has been reaped.
        let (sender, receiver) = oneshot::channel();
        let (accepted, acceptance) = oneshot::channel();
        tokio::spawn(async move {
            let mut starts = tokio::task::JoinSet::new();
            for id in active {
                let app = app.clone();
                starts.spawn(async move { app.activate(&id).await });
            }
            while starts.join_next().await.is_some() {}
            if sender.send(app.clone()).is_err() || acceptance.await.is_err() {
                let _ = app.shutdown().await;
            }
        });
        let app = receiver.await.map_err(|_| {
            error(
                ErrorKind::Unavailable,
                "application startup supervision stopped",
            )
        })?;
        // A delivered value can still be dropped inside an unpolled receiver. Only this
        // acknowledgment transfers lifecycle responsibility to the returned host.
        let _ = accepted.send(());
        Ok(app)
    }
    fn output<T: serde::Serialize>(&self, value: T) -> Result<T> {
        crate::agent_contract::check_serialized_size(&value, self.inner.limits.max_output_bytes)?;
        Ok(value)
    }
    pub fn limits(&self) -> &Limits {
        &self.inner.limits
    }
    /// Transport adapters supply authenticated workspace identity here, never model arguments.
    pub fn scope(&self, workspace: &str, request: &str, run: Option<&str>) -> Result<Scope> {
        validate_id(workspace)?;
        validate_id(request)?;
        if let Some(run) = run {
            validate_id(run)?;
        }
        let scope = Scope {
            workspace_id: workspace.into(),
            request_id: request.into(),
            run_id: run.map(Into::into),
        };
        scope.validate()?;
        Ok(scope)
    }
    pub fn offering(&self) -> Result<Offering> {
        self.output(lock(&self.inner.catalog)?.snapshot())
    }
    pub fn providers(&self) -> Result<Vec<ProviderState>> {
        self.output(lock(&self.inner.catalog)?.providers().cloned().collect())
    }
    pub fn subscribe(&self) -> broadcast::Receiver<ApplicationEvent> {
        self.inner.events.subscribe()
    }
    pub async fn activate(&self, id: &str) -> Result<()> {
        self.lifecycle(id, true).await
    }
    pub async fn deactivate(&self, id: &str) -> Result<()> {
        self.lifecycle(id, false).await
    }
    async fn lifecycle(&self, id: &str, activate: bool) -> Result<()> {
        let control = self
            .inner
            .providers
            .get(id)
            .ok_or_else(|| error(ErrorKind::Unavailable, "provider is not configured"))?;
        // Waiting callers own no task/effects. Only one admitted task per provider can
        // exist, and its owned permit survives cancellation of the caller's wait.
        let permit = control.lifecycle.clone().lock_owned().await;
        let app = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let _permit = permit;
            if activate {
                app.activate_owned(&id).await
            } else {
                app.deactivate_owned(&id).await
            }
        })
        .await
        .map_err(|_| {
            error(
                ErrorKind::Unavailable,
                "provider lifecycle supervision stopped",
            )
        })?
    }
    async fn activate_owned(&self, id: &str) -> Result<()> {
        let control = self
            .inner
            .providers
            .get(id)
            .ok_or_else(|| error(ErrorKind::Unavailable, "provider is not configured"))?;
        let old = {
            let state = lock(&self.inner.admission)?;
            if state.closed {
                return Err(error(
                    ErrorKind::Unavailable,
                    "application is shutting down",
                ));
            }
            // Fence an old generation before waiting for its cleanup.
            let mut catalog = lock(&self.inner.catalog)?;
            catalog.deactivate(id)?;
            for job in state.jobs.values().filter(|j| j.instance_id == id) {
                job.cancellation.cancel();
            }
            state.workers.get(id).cloned()
        };
        if let Some(old) = old {
            old.shutdown().await?;
            lock(&self.inner.admission)?.workers.remove(id);
        }
        {
            let state = lock(&self.inner.admission)?;
            if state.closed {
                return Err(error(
                    ErrorKind::Unavailable,
                    "application is shutting down",
                ));
            }
            lock(&self.inner.catalog)?.activate(id)?;
        }
        let worker = WorkerHandle::start(
            control.factory.clone(),
            self.inner.repositories.clone(),
            self.inner.catalog.clone(),
            self.inner.limits.clone(),
            self.inner.slots.clone(),
        )
        .await?;
        let publish = {
            let mut state = lock(&self.inner.admission)?;
            if state.closed {
                lock(&self.inner.catalog)?.deactivate(id)?;
                // Retain the handle while cleanup runs, including its terminal error if
                // close fails. Shutdown waits this lifecycle permit and observes that error.
                state.workers.insert(id.into(), worker.clone());
                false
            } else {
                state.workers.insert(id.into(), worker.clone());
                true
            }
        };
        if !publish {
            worker.shutdown().await?;
            lock(&self.inner.admission)?.workers.remove(id);
            return Err(error(
                ErrorKind::Unavailable,
                "application is shutting down",
            ));
        }
        Ok(())
    }
    async fn deactivate_owned(&self, id: &str) -> Result<()> {
        let worker = {
            let state = lock(&self.inner.admission)?;
            let mut catalog = lock(&self.inner.catalog)?;
            catalog.deactivate(id)?;
            for job in state.jobs.values().filter(|j| j.instance_id == id) {
                job.cancellation.cancel();
            }
            state.workers.get(id).cloned()
        };
        if let Some(worker) = worker {
            worker.shutdown().await?;
            lock(&self.inner.admission)?.workers.remove(id);
        }
        Ok(())
    }
    pub fn submit_manual(&self, scope: &Scope, command: FetchCommand) -> Result<JobReceipt> {
        scope.validate()?;
        command.validate()?;
        let offering = {
            let state = lock(&self.inner.admission)?;
            if state.closed {
                return Err(error(
                    ErrorKind::Unavailable,
                    "application is shutting down",
                ));
            }
            let catalog = lock(&self.inner.catalog)?;
            let provider = catalog
                .get(command.instance_id())
                .ok_or_else(|| error(ErrorKind::Unavailable, "provider is not configured"))?;
            if !provider.active {
                return Err(error(ErrorKind::Deactivated, "provider is deactivated"));
            }
            if !provider.available {
                return Err(error(ErrorKind::Unavailable, "provider is unavailable"));
            }
            catalog.snapshot()
        };
        self.submit(scope, &offering, command)
    }
    pub fn submit(
        &self,
        scope: &Scope,
        offering: &Offering,
        command: FetchCommand,
    ) -> Result<JobReceipt> {
        scope.validate()?;
        command.validate()?;
        crate::agent_contract::check_serialized_size(&command, self.inner.limits.max_input_bytes)?;
        let mut state = lock(&self.inner.admission)?;
        if state.closed {
            return Err(error(
                ErrorKind::Unavailable,
                "application is shutting down",
            ));
        }
        if state
            .jobs
            .values()
            .filter(|j| !j.status.borrow().is_terminal())
            .count()
            >= self.inner.bounds.max_pending_jobs
        {
            return Err(error(
                ErrorKind::ResourceLimit,
                "application job registry is full",
            ));
        }
        lock(&self.inner.catalog)?.authorize(offering, &command)?;
        let instance_id = command.instance_id().to_string();
        let worker = state
            .workers
            .get(&instance_id)
            .ok_or_else(|| error(ErrorKind::Unavailable, "provider is unavailable"))?;
        let id = lock(&self.inner.ids)?.next_id();
        self.scope(&scope.workspace_id, &id, scope.run_id.as_deref())?;
        if state.jobs.contains_key(&id) {
            return Err(error(
                ErrorKind::Conflict,
                "identifier source returned a duplicate job ID",
            ));
        }
        let job = worker.submit(offering.clone(), scope.clone(), command)?;
        let receipt = JobReceipt {
            id: id.clone(),
            scope: scope.clone(),
        };
        let initial = JobStatus {
            receipt: receipt.clone(),
            state: JobState::Queued,
            fetch_id: None,
            error: None,
        };
        let (status, _) = watch::channel(initial.clone());
        state.jobs.insert(
            id.clone(),
            RegisteredJob {
                instance_id,
                cancellation: job.cancellation(),
                status: status.clone(),
            },
        );
        let _ = self.inner.events.send(ApplicationEvent { job: initial });
        let app = self.clone();
        tokio::spawn(async move {
            app.supervise(id, job, status).await;
        });
        Ok(receipt)
    }
    async fn supervise(&self, id: String, job: JobHandle, status: watch::Sender<JobStatus>) {
        let mut changes = job.subscribe_state();
        let result = job.wait();
        tokio::pin!(result);
        let result = loop {
            tokio::select! {
                result = &mut result => break result,
                changed = changes.changed() => {
                    if changed.is_ok() && *changes.borrow_and_update() == JobState::Running {
                        let mut update = status.borrow().clone(); update.state = JobState::Running;
                        status.send_replace(update.clone()); let _ = self.inner.events.send(ApplicationEvent { job: update });
                    }
                }
            }
        };
        let mut terminal = status.borrow().clone();
        match result {
            Ok(result) => {
                terminal.state = result.state();
                terminal.error = result.error.clone();
                match self
                    .store(move |store| store.record_fetch(&result).map(|reference| reference.id))
                    .await
                {
                    Ok(id) => terminal.fetch_id = Some(id),
                    Err(error) => {
                        terminal.state = JobState::Failed;
                        terminal.error = Some(error);
                    }
                }
            }
            Err(error) => {
                terminal.state = JobState::Failed;
                terminal.error = Some(error);
            }
        }
        // Publishing and pruning share admission, so a newly admitted job cannot exceed retention.
        if let Ok(mut state) = self.inner.admission.lock() {
            status.send_replace(terminal.clone());
            state.terminals.push_back(id);
            while state.terminals.len() > self.inner.bounds.max_terminal_jobs {
                if let Some(old) = state.terminals.pop_front() {
                    state.jobs.remove(&old);
                }
            }
        } else {
            status.send_replace(terminal.clone());
        }
        let _ = self.inner.events.send(ApplicationEvent { job: terminal });
    }
    fn receiver(&self, scope: &Scope, id: &str) -> Result<watch::Receiver<JobStatus>> {
        scope.validate()?;
        validate_id(id)?;
        let state = lock(&self.inner.admission)?;
        let job = state
            .jobs
            .get(id)
            .ok_or_else(|| error(ErrorKind::MissingData, "job is not retained"))?;
        if job.status.borrow().receipt.scope.workspace_id != scope.workspace_id {
            return Err(error(
                ErrorKind::ScopeMismatch,
                "job belongs to another workspace",
            ));
        }
        Ok(job.status.subscribe())
    }
    pub fn status(&self, scope: &Scope, id: &str) -> Result<JobStatus> {
        self.output(self.receiver(scope, id)?.borrow().clone())
    }
    pub async fn wait(&self, scope: &Scope, id: &str) -> Result<JobStatus> {
        let mut status = self.receiver(scope, id)?;
        loop {
            let value = status.borrow_and_update().clone();
            if value.is_terminal() {
                return self.output(value);
            }
            status
                .changed()
                .await
                .map_err(|_| error(ErrorKind::Unavailable, "job supervision stopped"))?;
        }
    }
    pub fn cancel(&self, scope: &Scope, id: &str) -> Result<()> {
        scope.validate()?;
        validate_id(id)?;
        let state = lock(&self.inner.admission)?;
        let job = state
            .jobs
            .get(id)
            .ok_or_else(|| error(ErrorKind::MissingData, "job is not retained"))?;
        if job.status.borrow().receipt.scope.workspace_id != scope.workspace_id {
            return Err(error(
                ErrorKind::ScopeMismatch,
                "job belongs to another workspace",
            ));
        }
        job.cancellation.cancel();
        Ok(())
    }
    pub async fn shutdown(&self) -> Result<()> {
        let mut completion = {
            let mut state = lock(&self.inner.admission)?;
            if let Some(completion) = &state.shutdown {
                completion.clone()
            } else {
                state.closed = true;
                let mut catalog = lock(&self.inner.catalog)?;
                for id in self.inner.providers.keys() {
                    catalog.deactivate(id)?;
                }
                let pending = state
                    .jobs
                    .values()
                    .map(|job| {
                        job.cancellation.cancel();
                        job.status.subscribe()
                    })
                    .collect();
                let (sender, completion) = watch::channel(None);
                state.shutdown = Some(completion.clone());
                let app = self.clone();
                // A single host-owned shutdown task retains all stop and job completion
                // waits, even if every caller drops its shutdown future.
                tokio::spawn(async move {
                    let result = app.shutdown_owned(pending).await;
                    sender.send_replace(Some(result));
                });
                completion
            }
        };
        loop {
            if let Some(result) = completion.borrow_and_update().clone() {
                return result;
            }
            completion.changed().await.map_err(|_| {
                error(
                    ErrorKind::Unavailable,
                    "application shutdown supervision stopped",
                )
            })?;
        }
    }
    async fn shutdown_owned(&self, mut pending: Vec<watch::Receiver<JobStatus>>) -> Result<()> {
        let mut stops = tokio::task::JoinSet::new();
        for id in self.inner.providers.keys() {
            let app = self.clone();
            let id = id.clone();
            stops.spawn(async move { app.deactivate(&id).await });
        }
        let mut failure = None;
        while let Some(result) = stops.join_next().await {
            if let Err(error) = result.unwrap_or_else(|_| {
                Err(error(
                    ErrorKind::Unavailable,
                    "provider cleanup task failed",
                ))
            }) {
                failure.get_or_insert(error);
            }
        }
        for status in &mut pending {
            while !status.borrow_and_update().is_terminal() {
                status
                    .changed()
                    .await
                    .map_err(|_| error(ErrorKind::Unavailable, "job supervision stopped"))?;
            }
        }
        failure.map_or(Ok(()), Err)
    }
    async fn store<T: Send + serde::Serialize + 'static>(
        &self,
        action: impl FnOnce(&mut dyn ApplicationStore) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = self.inner.store.clone();
        let max = self.inner.limits.max_output_bytes;
        tokio::task::spawn_blocking(move || {
            let value = action(&mut **lock(&store)?)?;
            crate::agent_contract::check_serialized_size(&value, max)?;
            Ok(value)
        })
        .await
        .map_err(|_| error(ErrorKind::Storage, "application storage task failed"))?
    }
    pub async fn read_fetch(&self, scope: &Scope, id: &str) -> Result<FetchReference> {
        scope.validate()?;
        validate_id(id)?;
        let scope = scope.clone();
        let id = id.to_string();
        self.store(move |s| s.read_fetch(&scope, &id)).await
    }
    pub async fn create_dataset(
        &self,
        scope: &Scope,
        id: &str,
        projection: DatasetProjection,
    ) -> Result<DatasetHeader> {
        crate::agent_contract::check_serialized_size(
            &projection,
            self.inner.limits.max_input_bytes,
        )?;
        scope.validate()?;
        validate_id(id)?;
        let scope = scope.clone();
        let id = id.to_string();
        self.store(move |s| s.create_dataset(&scope, &id, projection))
            .await
    }
    pub async fn dataset_header(&self, scope: &Scope, id: &str) -> Result<DatasetHeader> {
        scope.validate()?;
        validate_id(id)?;
        let scope = scope.clone();
        let id = id.to_string();
        self.store(move |s| s.dataset_header(&scope, &id)).await
    }
    pub async fn read_dataset(
        &self,
        scope: &Scope,
        id: &str,
        page: PageRequest,
    ) -> Result<DatasetPage> {
        scope.validate()?;
        validate_id(id)?;
        let scope = scope.clone();
        let id = id.to_string();
        self.store(move |s| s.read_dataset(&scope, &id, page)).await
    }
    pub async fn read_document(
        &self,
        scope: &Scope,
        id: &str,
        offset: usize,
        length: usize,
    ) -> Result<DocumentRead> {
        scope.validate()?;
        validate_id(id)?;
        let scope = scope.clone();
        let id = id.to_string();
        self.store(move |s| s.read_document(&scope, &id, offset, length))
            .await
    }
    pub async fn select_candidate(
        &self,
        scope: &Scope,
        id: &str,
        observation_id: i64,
    ) -> Result<lugus_financial::resolution::catalog::CatalogSelection> {
        scope.validate()?;
        validate_id(id)?;
        let scope = scope.clone();
        let id = id.to_string();
        self.store(move |s| s.select_candidate(&scope, &id, observation_id))
            .await
    }
    pub async fn open_view(&self, scope: &Scope, request: OpenViewRequest) -> Result<ViewReceipt> {
        crate::agent_contract::check_serialized_size(&request, self.inner.limits.max_input_bytes)?;
        validate_id(&request.dataset_id)?;
        scope.validate()?;
        let scope = scope.clone();
        self.store(move |s| s.open_view(&scope, &request)).await
    }
    pub async fn read_view(&self, scope: &Scope, id: &str) -> Result<ViewReceipt> {
        scope.validate()?;
        validate_id(id)?;
        let scope = scope.clone();
        let id = id.to_string();
        self.store(move |s| s.read_view(&scope, &id)).await
    }
    pub async fn report_presentation(
        &self,
        scope: &Scope,
        result: PresentationResult,
    ) -> Result<()> {
        crate::agent_contract::check_serialized_size(&result, self.inner.limits.max_input_bytes)?;
        validate_id(&result.view_id)?;
        scope.validate()?;
        let scope = scope.clone();
        self.store(move |s| s.report_presentation(&scope, &result))
            .await
    }
}

fn validate_id(id: &str) -> Result<()> {
    if id.trim().is_empty() || id.len() > Scope::MAX_ID_BYTES || id.chars().any(char::is_control) {
        Err(error(
            ErrorKind::InvalidInput,
            "invalid reference identifier",
        ))
    } else {
        Ok(())
    }
}
