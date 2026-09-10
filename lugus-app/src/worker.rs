//! Dedicated per-instance threads isolate mutable providers and synchronous repositories.
mod budget;
mod recording;
use crate::{
    AppError, Catalog, ErrorKind, FetchCommand, JobState, Limits, ManagedProvider, Offering,
    ProviderFactory, ProviderIdentity, RepositoryFactory, Result, Scope, WorkerRepository,
};
use budget::{BoundedProvider, Budget, cancelled};
use lugus_financial::{
    application,
    resolution::{application as resolution_app, catalog::ResolutionOutcome},
    storage::DocumentObservation,
};
use recording::Recording;
pub use recording::{RunKind, RunReceipt};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::{
    sync::{Semaphore, mpsc, oneshot, watch},
    time::Instant,
};

/// Clone once per worker to share the host's bounded active-job capacity.
#[derive(Clone)]
pub struct WorkerSlots {
    semaphore: Arc<Semaphore>,
    capacity: usize,
}
impl WorkerSlots {
    pub fn new(capacity: usize) -> Result<Self> {
        if !(1..=Limits::MAX_CONCURRENT_JOBS).contains(&capacity) {
            return Err(AppError::new(
                ErrorKind::InvalidInput,
                "invalid concurrent job capacity",
                false,
            ));
        }
        Ok(Self {
            semaphore: Arc::new(Semaphore::new(capacity)),
            capacity,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchProvenance {
    pub scope: Scope,
    pub provider: ProviderIdentity,
    pub repository_id: String,
    pub command: FetchCommand,
    pub runs: Vec<RunReceipt>,
    pub document: Option<DocumentObservation>,
    #[serde(default)]
    pub instrument_observation: Option<lugus_financial::instruments::InstrumentObservation>,
    #[serde(default)]
    pub binding_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResult {
    pub provenance: FetchProvenance,
    pub error: Option<AppError>,
}
impl FetchResult {
    pub fn state(&self) -> JobState {
        match self.error.as_ref().map(|e| e.kind) {
            None => JobState::Succeeded,
            Some(ErrorKind::Cancelled) => JobState::Cancelled,
            _ => JobState::Failed,
        }
    }
}
#[derive(Clone)]
pub struct Cancellation {
    sender: watch::Sender<bool>,
}
impl Cancellation {
    pub fn cancel(&self) {
        self.sender.send_replace(true);
    }
}
pub struct JobHandle {
    cancellation: Cancellation,
    state: watch::Receiver<JobState>,
    result: oneshot::Receiver<FetchResult>,
}
impl JobHandle {
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
    pub fn cancellation(&self) -> Cancellation {
        self.cancellation.clone()
    }
    pub fn state(&self) -> JobState {
        *self.state.borrow()
    }
    pub fn subscribe_state(&self) -> watch::Receiver<JobState> {
        self.state.clone()
    }
    pub async fn wait(self) -> Result<FetchResult> {
        self.result
            .await
            .map_err(|_| unavailable("worker stopped before returning a result"))
    }
}
struct Job {
    offering: Offering,
    provenance: FetchProvenance,
    generation: u64,
    deadline: Instant,
    cancel: watch::Receiver<bool>,
    state: watch::Sender<JobState>,
    result: oneshot::Sender<FetchResult>,
}
impl Job {
    fn finish(self, error: Option<AppError>) {
        let result = FetchResult {
            provenance: self.provenance,
            error,
        };
        self.state.send_replace(result.state());
        let _ = self.result.send(result);
    }
}
struct Control {
    sender: mpsc::Sender<Job>,
    shutdown: watch::Sender<bool>,
    done: watch::Receiver<Option<Result<()>>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl Drop for Control {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
    }
}
#[derive(Clone)]
pub struct WorkerHandle {
    control: Arc<Control>,
    catalog: Arc<Mutex<Catalog>>,
    identity: ProviderIdentity,
    repository_id: String,
    generation: u64,
    limits: Limits,
}
fn unavailable(message: &str) -> AppError {
    AppError::new(ErrorKind::Unavailable, message, false)
}
fn cancelled_error() -> AppError {
    AppError::new(ErrorKind::Cancelled, "provider operation cancelled", false)
}
fn lock_catalog(catalog: &Mutex<Catalog>) -> Result<std::sync::MutexGuard<'_, Catalog>> {
    catalog
        .lock()
        .map_err(|_| unavailable("provider catalog is unavailable"))
}
impl WorkerHandle {
    /// Initialize repository schemas before starting any workers. All workers of a host share `slots`.
    pub async fn start(
        factory: Arc<dyn ProviderFactory>,
        repositories: Arc<dyn RepositoryFactory>,
        catalog: Arc<Mutex<Catalog>>,
        limits: Limits,
        slots: WorkerSlots,
    ) -> Result<Self> {
        limits.validate()?;
        if slots.capacity > limits.max_concurrent_jobs {
            return Err(AppError::new(
                ErrorKind::InvalidInput,
                "shared slots exceed configured job capacity",
                false,
            ));
        }
        let identity = factory.identity();
        let expected_generation = lock_catalog(&catalog)?
            .get(&identity.instance_id)
            .ok_or_else(|| unavailable("provider is not configured"))?
            .generation;
        let (sender, receiver) = mpsc::channel(limits.queue_capacity);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let (done_tx, done) = watch::channel(None);
        let (ready_tx, ready) = oneshot::channel();
        let thread_catalog = catalog.clone();
        let thread_limits = limits.clone();
        let thread_identity = identity.clone();
        let thread = std::thread::Builder::new()
            .name(format!("provider-{}", identity.instance_id))
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let result = match runtime {
                    Ok(runtime) => runtime.block_on(async move {
                        let setup = async {
                            let repo = repositories.open().map_err(AppError::from)?;
                            let repository_id =
                                repo.repository_identity().map_err(AppError::from)?;
                            let mut provider = factory
                                .start(&thread_limits)
                                .await
                                .map_err(AppError::from)?;
                            let registration = (|| {
                                if provider.identity() != &thread_identity {
                                    return Err(unavailable("provider identity mismatch"));
                                }
                                let mut catalog = lock_catalog(&thread_catalog)?;
                                if catalog
                                    .get(&thread_identity.instance_id)
                                    .map(|p| p.generation)
                                    != Some(expected_generation)
                                {
                                    return Err(AppError::new(
                                        ErrorKind::Conflict,
                                        "provider generation changed during startup",
                                        false,
                                    ));
                                }
                                catalog.restart(
                                    provider.identity().clone(),
                                    provider.capabilities().clone(),
                                )
                            })();
                            let generation = match registration {
                                Ok(generation) => generation,
                                Err(error) => {
                                    provider.close().await.map_err(AppError::from)?;
                                    return Err(error);
                                }
                            };
                            Ok((repo, repository_id, provider, generation))
                        }
                        .await;
                        match setup {
                            Ok((mut repo, repository_id, mut provider, generation)) => {
                                if ready_tx.send(Ok((repository_id, generation))).is_err() {
                                    mark_unavailable(&thread_catalog, &thread_identity, generation);
                                    return provider.close().await.map_err(AppError::from);
                                }
                                run_worker(
                                    &mut *provider,
                                    &mut *repo,
                                    receiver,
                                    shutdown_rx,
                                    &thread_catalog,
                                    generation,
                                    thread_limits,
                                    slots.semaphore,
                                )
                                .await
                            }
                            Err(error) => {
                                let _ = ready_tx.send(Err(error.clone()));
                                Err(error)
                            }
                        }
                    }),
                    Err(_) => {
                        let error = unavailable("could not create provider runtime");
                        let _ = ready_tx.send(Err(error.clone()));
                        Err(error)
                    }
                };
                done_tx.send_replace(Some(result));
            })
            .map_err(|_| unavailable("could not create provider thread"))?;
        let startup = ready
            .await
            .map_err(|_| unavailable("provider startup thread stopped"));
        let (repository_id, generation) = match startup.and_then(|result| result) {
            Ok(ready) => ready,
            Err(error) => {
                let _ = tokio::task::spawn_blocking(move || thread.join()).await;
                return Err(error);
            }
        };
        Ok(Self {
            control: Arc::new(Control {
                sender,
                shutdown,
                done,
                thread: Mutex::new(Some(thread)),
            }),
            catalog,
            identity,
            repository_id,
            generation,
            limits,
        })
    }
    pub fn identity(&self) -> &ProviderIdentity {
        &self.identity
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }
    pub fn submit(
        &self,
        offering: Offering,
        scope: Scope,
        command: FetchCommand,
    ) -> Result<JobHandle> {
        scope.validate()?;
        command.validate()?;
        if *self.control.shutdown.borrow() {
            return Err(unavailable("worker is shutting down"));
        }
        if command.instance_id() != self.identity.instance_id {
            return Err(AppError::new(
                ErrorKind::InvalidInput,
                "command targets a different worker",
                false,
            ));
        }
        if serde_json::to_vec(&command)
            .map_err(|_| AppError::new(ErrorKind::InvalidInput, "invalid fetch command", false))?
            .len()
            > self.limits.max_input_bytes
        {
            return Err(AppError::new(
                ErrorKind::ResourceLimit,
                "fetch command exceeds input budget",
                false,
            ));
        }
        let authorization = lock_catalog(&self.catalog)?.authorize(&offering, &command)?;
        if authorization.generation != self.generation {
            return Err(AppError::new(
                ErrorKind::Conflict,
                "worker generation is stale",
                false,
            ));
        }
        let (cancel_tx, cancel) = watch::channel(false);
        let (state_tx, state) = watch::channel(JobState::Queued);
        let (result_tx, result) = oneshot::channel();
        let job = Job {
            offering,
            provenance: FetchProvenance {
                scope,
                provider: self.identity.clone(),
                repository_id: self.repository_id.clone(),
                command,
                runs: vec![],
                document: None,
                instrument_observation: None,
                binding_id: None,
            },
            generation: self.generation,
            deadline: Instant::now() + self.limits.operation_timeout,
            cancel,
            state: state_tx,
            result: result_tx,
        };
        self.control
            .sender
            .try_send(job)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    AppError::new(ErrorKind::ResourceLimit, "provider queue is full", true)
                }
                mpsc::error::TrySendError::Closed(_) => unavailable("provider worker is stopped"),
            })?;
        Ok(JobHandle {
            cancellation: Cancellation { sender: cancel_tx },
            state,
            result,
        })
    }
    /// Cancels exchanges, drains queued jobs, explicitly reaps the child, and joins its thread.
    pub async fn shutdown(&self) -> Result<()> {
        self.control.shutdown.send_replace(true);
        let mut done = self.control.done.clone();
        let result = loop {
            if let Some(result) = done.borrow_and_update().clone() {
                break result;
            }
            done.changed()
                .await
                .map_err(|_| unavailable("provider worker exited unexpectedly"))?;
        };
        let thread = self
            .control
            .thread
            .lock()
            .map_err(|_| unavailable("worker join is unavailable"))?
            .take();
        if let Some(thread) = thread {
            tokio::task::spawn_blocking(move || thread.join())
                .await
                .map_err(|_| unavailable("worker join failed"))?
                .map_err(|_| unavailable("provider worker panicked"))?;
        }
        result
    }
}
fn mark_unavailable(catalog: &Mutex<Catalog>, identity: &ProviderIdentity, generation: u64) {
    if let Ok(mut catalog) = catalog.lock()
        && catalog.get(&identity.instance_id).map(|p| p.generation) == Some(generation)
    {
        let _ = catalog.set_available(&identity.instance_id, false);
    }
}
#[allow(clippy::too_many_arguments)]
async fn run_worker(
    provider: &mut dyn ManagedProvider,
    repo: &mut dyn WorkerRepository,
    mut queue: mpsc::Receiver<Job>,
    mut shutdown: watch::Receiver<bool>,
    catalog: &Mutex<Catalog>,
    generation: u64,
    limits: Limits,
    slots: Arc<Semaphore>,
) -> Result<()> {
    loop {
        let job = tokio::select! {
            biased;
            _ = cancelled(&mut shutdown) => { queue.close(); break; },
            job = queue.recv() => match job { Some(job) => job, None => break },
        };
        execute_job(
            provider,
            repo,
            job,
            shutdown.clone(),
            catalog,
            &limits,
            &slots,
        )
        .await;
        if !provider.is_running() {
            mark_unavailable(catalog, provider.identity(), generation);
        }
    }
    mark_unavailable(catalog, provider.identity(), generation);
    while let Some(job) = queue.recv().await {
        job.finish(Some(cancelled_error()));
    }
    provider.close().await.map_err(AppError::from)
}
async fn execute_job(
    provider: &mut dyn ManagedProvider,
    repo: &mut dyn WorkerRepository,
    mut job: Job,
    mut shutdown: watch::Receiver<bool>,
    catalog: &Mutex<Catalog>,
    limits: &Limits,
    slots: &Arc<Semaphore>,
) {
    let permit = tokio::select! {
        biased;
        _ = cancelled(&mut shutdown) => Err(cancelled_error()),
        _ = cancelled(&mut job.cancel) => Err(cancelled_error()),
        _ = tokio::time::sleep_until(job.deadline) => Err(AppError::new(ErrorKind::Timeout, "queued job deadline exceeded", false)),
        permit = slots.clone().acquire_owned() => permit.map_err(|_| unavailable("job capacity is closed")),
    };
    let _permit = match permit {
        Ok(permit) => permit,
        Err(error) => {
            job.finish(Some(error));
            return;
        }
    };
    let authorized = lock_catalog(catalog).and_then(|catalog| {
        // Admission closure and cancellation are published under this same catalog lock.
        // Capacity acquisition may precede that closure, so recheck after acquiring the lock.
        if *job.cancel.borrow() || *shutdown.borrow() {
            return Err(cancelled_error());
        }
        catalog.authorize(&job.offering, &job.provenance.command)
    });
    let error = match authorized {
        Ok(auth) if auth.generation == job.generation && provider.identity() == &auth.identity => {
            None
        }
        Ok(_) => Some(AppError::new(
            ErrorKind::Conflict,
            "worker generation changed while queued",
            false,
        )),
        Err(error) => Some(error),
    };
    if error.is_some() {
        job.finish(error);
        return;
    }
    if !provider.is_running() {
        job.finish(Some(unavailable("provider process is stopped")));
        return;
    }
    job.state.send_replace(JobState::Running);
    let mut recording = Recording {
        inner: repo,
        runs: vec![],
        document: None,
        instrument_observation: None,
        read_limits: lugus_financial::storage::bounded::ReadLimits {
            max_items: limits.max_items_per_fetch,
            max_bytes: limits.max_bytes_per_fetch,
        },
        read_limited: std::cell::Cell::new(false),
        finalization_failed: std::cell::Cell::new(false),
        protocol_failed: false,
        resolution_failure: None,
    };
    let mut bounded = BoundedProvider {
        inner: provider,
        budget: Budget::new(limits.clone(), job.deadline, job.cancel.clone(), shutdown),
    };
    let outcome = fetch(
        &mut recording,
        &mut bounded,
        &job.provenance.command,
        limits,
    )
    .await;
    // Finalization may replace a protocol error with a storage error; cleanup is still required.
    let protocol_failed = recording.protocol_failed
        || outcome
            .as_ref()
            .is_err_and(|error| error.kind == lugus_financial::error::ErrorKind::Protocol);
    // Only wrapper-caused errors are cancellation/resource failures. Storage finalization errors win.
    let mut error = match outcome {
        Ok(()) => None,
        Err(_) if recording.finalization_failed.get() => Some(AppError::new(
            ErrorKind::Storage,
            "could not finalize provider run",
            false,
        )),
        Err(error) if error.kind == lugus_financial::error::ErrorKind::Persistence => {
            Some(AppError::from(error))
        }
        Err(_) if bounded.budget.cause.is_some() => bounded.budget.cause.take(),
        Err(_) if recording.resolution_failure.is_some() => recording.resolution_failure.take(),
        Err(_) if recording.read_limited.get() => Some(AppError::new(
            ErrorKind::ResourceLimit,
            "resolution evidence exceeds read budget",
            false,
        )),
        Err(error) => bounded
            .budget
            .cause
            .take()
            .or_else(|| Some(AppError::from(error))),
    };
    job.provenance.runs = recording.runs;
    job.provenance.document = recording.document;
    job.provenance.instrument_observation = recording.instrument_observation;
    let interrupted = error
        .as_ref()
        .into_iter()
        .chain(bounded.budget.cause.as_ref())
        .any(|e| matches!(e.kind, ErrorKind::Cancelled | ErrorKind::Timeout));
    if protocol_failed || interrupted || !bounded.inner.is_running() {
        let closed = bounded.inner.close().await;
        mark_unavailable(catalog, bounded.inner.identity(), job.generation);
        if let Err(close_error) = closed
            && !error.as_ref().is_some_and(|e| e.kind == ErrorKind::Storage)
        {
            error = Some(close_error.into());
        }
    }
    job.finish(error);
}
async fn fetch(
    repo: &mut Recording<'_>,
    provider: &mut BoundedProvider<'_>,
    command: &FetchCommand,
    limits: &Limits,
) -> lugus_financial::error::Result<()> {
    use lugus_financial::error::{Error, ErrorKind as FinancialKind};
    let resolution = match command {
        FetchCommand::Filings { query, .. } => {
            return application::ingest_filings(repo, provider, query)
                .await
                .map(|_| ());
        }
        FetchCommand::Facts { query, .. } => {
            return application::ingest_facts(repo, provider, query)
                .await
                .map(|_| ());
        }
        FetchCommand::InstrumentLookup { query, .. } => {
            use lugus_financial::instruments::{InstrumentProvider, InstrumentRepository};
            let metadata = provider.lookup_instrument(query).await?;
            repo.save_instrument_observation(provider.inner.identity(), query, &metadata)?;
            return Ok(());
        }
        FetchCommand::Prices { query, .. } => {
            return application::market::ingest_prices(repo, provider, query)
                .await
                .map(|_| ());
        }
        FetchCommand::Document { source_url, .. } => {
            return application::retrieve_document(
                repo,
                provider,
                source_url,
                limits.max_document_bytes,
            )
            .await
            .map(|_| ());
        }
        FetchCommand::Resolve { input, .. } => {
            resolution_app::resolve_input(
                repo,
                provider,
                input,
                limits.max_items_per_fetch.min(100),
                1000,
            )
            .await?
        }
        FetchCommand::Lookup { request, .. } => {
            resolution_app::lookup_and_store(repo, provider, request).await?
        }
    };
    if let ResolutionOutcome::Incomplete { failure, .. } = resolution.outcome {
        return Err(match failure {
            Some(failure) => Error {
                kind: failure.kind,
                message: failure.message,
                retry_after_seconds: failure.retry_after_seconds,
            },
            None => Error::new(FinancialKind::Unavailable, "resolution did not complete"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod dispatch_tests;
