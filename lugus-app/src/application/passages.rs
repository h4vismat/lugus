//! Supervised local text preparation. No extraction holds the application store mutex.
use super::*;
use crate::passages::{
    CreatePassageRequest, HtmlTextExtractor, Passage, PassageSource, TextExtractor, TextLimits,
    TextPage, TextRepresentation,
};
use std::sync::atomic::AtomicBool;

pub struct TextPreparationOptions {
    pub limits: TextLimits,
    pub max_concurrent_preparations: usize,
    /// Trusted adapters must honor finite limits and cooperative cancellation, including cleanup.
    pub extractor: Arc<dyn TextExtractor>,
}
impl Default for TextPreparationOptions {
    fn default() -> Self {
        Self {
            limits: TextLimits::default(),
            max_concurrent_preparations: 2,
            extractor: Arc::new(HtmlTextExtractor),
        }
    }
}
#[derive(Clone)]
pub(super) struct PreparationJob {
    cancellation: Arc<AtomicBool>,
    publication: Arc<Mutex<()>>,
    pub done: watch::Receiver<bool>,
}
impl PreparationJob {
    pub fn cancel(&self) {
        // The fence covers only commit admission; cancellation never waits on storage.
        let _fence = self.publication.lock().unwrap_or_else(|e| e.into_inner());
        self.cancellation.store(true, Ordering::Release);
    }
    fn check(&self) -> Result<()> {
        if self.cancellation.load(Ordering::Acquire) {
            Err(error(ErrorKind::Cancelled, "text preparation cancelled"))
        } else {
            Ok(())
        }
    }
}
struct CancelPreparation(PreparationJob);
impl Drop for CancelPreparation {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
impl Application {
    pub fn text_limits(&self) -> &TextLimits {
        &self.inner.text_options.limits
    }
    pub async fn prepare_text(
        &self,
        scope: &Scope,
        dataset_id: &str,
    ) -> Result<TextRepresentation> {
        scope.validate()?;
        validate_id(dataset_id)?;
        let scope = scope.clone();
        let dataset_id = dataset_id.to_owned();
        let (sender, receiver) = oneshot::channel();
        let guard = {
            let mut admission = lock(&self.inner.admission)?;
            if admission.closed {
                return Err(error(ErrorKind::Unavailable, "application is shut down"));
            }
            if admission.preparations.len() >= self.inner.text_options.max_concurrent_preparations {
                return Err(error(
                    ErrorKind::ResourceLimit,
                    "text preparation capacity reached",
                ));
            }
            let id = admission.next_preparation;
            admission.next_preparation = id.checked_add(1).ok_or_else(|| {
                error(
                    ErrorKind::ResourceLimit,
                    "text preparation identifiers exhausted",
                )
            })?;
            let (done, completion) = watch::channel(false);
            let job = PreparationJob {
                cancellation: Arc::new(AtomicBool::new(false)),
                publication: Arc::new(Mutex::new(())),
                done: completion,
            };
            admission.preparations.insert(id, job.clone());
            let app = self.clone();
            let worker_job = job.clone();
            // Register before spawn/releasing admission. The owned supervisor outlives caller Drop.
            tokio::spawn(async move {
                let worker = app.clone();
                let result = tokio::task::spawn_blocking(move || {
                    worker.prepare_owned(&scope, &dataset_id, &worker_job)
                })
                .await
                .unwrap_or_else(|_| {
                    Err(error(
                        ErrorKind::Unavailable,
                        "text preparation worker failed",
                    ))
                });
                if let Ok(mut admission) = lock(&app.inner.admission) {
                    admission.preparations.remove(&id);
                }
                done.send_replace(true);
                let _ = sender.send(result);
            });
            CancelPreparation(job)
        };
        let result = receiver.await.map_err(|_| {
            error(
                ErrorKind::Unavailable,
                "text preparation supervision stopped",
            )
        })?;
        drop(guard);
        result
    }
    fn prepare_owned(
        &self,
        scope: &Scope,
        dataset_id: &str,
        job: &PreparationJob,
    ) -> Result<TextRepresentation> {
        job.check()?;
        let options = &self.inner.text_options;
        let max = self.inner.limits.max_output_bytes;
        let identity = options.extractor.identity();
        let input = lock(&self.inner.store)?
            .passage_store()?
            .load_text_preparation(scope, dataset_id, &identity, &options.limits, max)?;
        job.check()?;
        let result = match input {
            TextPreparation::Cached(header) => *header,
            TextPreparation::Input(input) => {
                let extracted = options
                    .extractor
                    .extract(
                        &input.bytes,
                        input.media_type(),
                        &options.limits,
                        &job.cancellation,
                    )
                    .map_err(|e| error(e.kind, "text extraction failed"))?;
                let prepared =
                    PreparedText::new(*input, extracted, &options.limits, &job.cancellation)?;
                {
                    let admission = lock(&self.inner.admission)?;
                    let _fence = lock(&job.publication)?;
                    job.check()?;
                    if admission.closed {
                        return Err(error(ErrorKind::Cancelled, "text preparation cancelled"));
                    }
                    // Commit is now admitted. Its bounded atomic effect may finish after
                    // cancellation, while shutdown asynchronously drains this worker.
                }
                lock(&self.inner.store)?
                    .passage_store_mut()?
                    .save_text_representation(scope, &prepared, max)?
            }
        };
        crate::agent_contract::check_serialized_size(&result, max)?;
        Ok(result)
    }
    pub async fn text_header(&self, scope: &Scope, id: &str) -> Result<TextRepresentation> {
        scope.validate()?;
        validate_id(id)?;
        let (scope, id, max) = (
            scope.clone(),
            id.to_owned(),
            self.inner.limits.max_output_bytes,
        );
        self.store(move |s| {
            s.passage_store()?
                .read_text_representation(&scope, &id, max)
        })
        .await
    }
    pub async fn read_text(
        &self,
        scope: &Scope,
        id: &str,
        start: usize,
        end: usize,
    ) -> Result<TextPage> {
        scope.validate()?;
        validate_id(id)?;
        let (scope, id, max, limits) = (
            scope.clone(),
            id.to_owned(),
            self.inner.limits.max_output_bytes,
            self.text_limits().clone(),
        );
        self.store(move |s| {
            s.passage_store()?
                .read_text_page(&scope, &id, start, end, &limits, max)
        })
        .await
    }
    pub async fn create_passage(
        &self,
        scope: &Scope,
        request: CreatePassageRequest,
    ) -> Result<Passage> {
        scope.validate()?;
        validate_id(&request.representation_id)?;
        crate::agent_contract::check_serialized_size(&request, self.inner.limits.max_input_bytes)?;
        let (scope, max, limits) = (
            scope.clone(),
            self.inner.limits.max_output_bytes,
            self.text_limits().clone(),
        );
        self.store(move |s| {
            s.passage_store_mut()?
                .create_passage(&scope, &request, &limits, max)
        })
        .await
    }
    pub async fn read_passage(&self, scope: &Scope, id: &str) -> Result<Passage> {
        scope.validate()?;
        validate_id(id)?;
        let (scope, id, max) = (
            scope.clone(),
            id.to_owned(),
            self.inner.limits.max_output_bytes,
        );
        self.store(move |s| s.passage_store()?.read_passage(&scope, &id, max))
            .await
    }
    pub async fn resolve_passage(&self, scope: &Scope, id: &str) -> Result<PassageSource> {
        scope.validate()?;
        validate_id(id)?;
        let (scope, id, max) = (
            scope.clone(),
            id.to_owned(),
            self.inner.limits.max_output_bytes,
        );
        self.store(move |s| s.passage_store()?.resolve_passage_sources(&scope, &id, max))
            .await
    }
}
