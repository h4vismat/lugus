use super::*;
use crate::comparison::*;
mod prepare;
mod supervisor;
pub(super) use supervisor::ComparisonExecution;
use supervisor::ComparisonStop;
impl Application {
    pub(crate) async fn comparison_effect<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut dyn ComparisonStore) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = self.inner.store.clone();
        tokio::task::spawn_blocking(move || f(lock(&store)?.comparison_store_mut()?))
            .await
            .map_err(|_| error(ErrorKind::Storage, "comparison storage task failed"))?
    }
    pub fn comparison_providers(&self) -> Result<serde_json::Value> {
        let offering = self.offering()?;
        let resolution: Vec<_> = offering
            .operations()
            .filter(|o| o.operation == Operation::Resolve)
            .map(|o| o.identity.clone())
            .collect();
        let facts: Vec<_> = offering
            .operations()
            .filter(|o| eligible_facts(o))
            .map(|o| o.identity.clone())
            .collect();
        Ok(serde_json::json!({"resolution":resolution,"facts":facts}))
    }
    pub async fn start_comparison(
        &self,
        scope: &Scope,
        request: ComparisonRequest,
    ) -> Result<ComparisonJob> {
        scope.validate()?;
        request.validate(chrono::Utc::now().date_naive())?;
        crate::agent_contract::check_serialized_size(&request, self.limits().max_input_bytes)?;
        if scope.request_id != request.request_id {
            return Err(error(ErrorKind::InvalidInput, "request identity mismatch"));
        }
        let (s, r) = (scope.clone(), request.clone());
        if let Some(j) = self
            .comparison_effect(move |store| store.lookup_request(&s, &r))
            .await?
        {
            return self.comparison_status(scope, &j.id).await;
        }
        let offering = self.offering()?;
        let providers = CapturedProviders {
            facts: choose(
                &offering,
                Operation::Facts,
                request.facts_instance.as_deref(),
            )?,
            resolution: choose(
                &offering,
                Operation::Resolve,
                request.resolution_instance.as_deref(),
            )?,
        };
        let app = self.clone();
        let s = scope.clone();
        let handle = tokio::runtime::Handle::current();
        let job = self
            .comparison_effect(move |store| {
                if let Some(j) = store.lookup_request(&s, &request)? {
                    return Ok(j);
                }
                let mut admission = lock(&app.inner.admission)?;
                if admission.closed || admission.comparisons.len() >= 4 {
                    return Err(error(
                        ErrorKind::ResourceLimit,
                        "comparison admission is closed or full",
                    ));
                }
                let lease = store.acquire(&s)?;
                store.recover_interrupted(&s, &lease)?;
                let job = store.begin(&s, &request, &lease, &providers)?;
                let (cancel, receiver) = watch::channel(false);
                let (done_sender, done) = watch::channel(false);
                admission.comparisons.insert(
                    job.id.clone(),
                    ComparisonExecution {
                        workspace: s.workspace_id.clone(),
                        cancel,
                        done,
                    },
                );
                let output = job.clone();
                let worker_app = app.clone();
                handle.spawn(async move {
                    supervisor::run(worker_app, s, job, offering, lease, receiver, done_sender)
                        .await;
                });
                Ok(output)
            })
            .await?;
        self.comparison_status(scope, &job.id).await
    }
    pub async fn comparison_status(&self, scope: &Scope, id: &str) -> Result<ComparisonJob> {
        let (s, id) = (scope.clone(), id.to_string());
        let app = self.clone();
        self.comparison_effect(move |store| {
            let mut j = store.job(&s, &id)?;
            if !j.state.terminal() {
                if lock(&app.inner.admission)?.comparisons.contains_key(&id) {
                    j.owner = ComparisonOwner::Local;
                } else {
                    match store.acquire(&s) {
                        Ok(lease) => {
                            store.recover_interrupted(&s, &lease)?;
                            j = store.job(&s, &id)?;
                        }
                        Err(e) if e.kind == ErrorKind::Conflict => {
                            j.owner = ComparisonOwner::External
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            crate::agent_contract::check_serialized_size(&j, app.limits().max_output_bytes)?;
            Ok(j)
        })
        .await
    }
    pub async fn cancel_comparison(&self, scope: &Scope, id: &str) -> Result<()> {
        let j = self.comparison_status(scope, id).await?;
        if j.state.terminal() {
            return Ok(());
        }
        let mut done = {
            let state = lock(&self.inner.admission)?;
            let job = state
                .comparisons
                .get(id)
                .filter(|j| j.workspace == scope.workspace_id)
                .ok_or_else(|| {
                    error(
                        ErrorKind::Conflict,
                        "comparison is owned by another process",
                    )
                })?;
            job.cancel.send_replace(true);
            job.done.clone()
        };
        while !*done.borrow_and_update() {
            done.changed()
                .await
                .map_err(|_| error(ErrorKind::Unavailable, "comparison cleanup stopped"))?;
        }
        Ok(())
    }
    pub async fn read_comparison(&self, s: &Scope, id: &str) -> Result<ComparisonRecord> {
        let (s, id) = (s.clone(), id.to_owned());
        self.comparison_effect(move |store| store.read(&s, &id))
            .await
    }
    pub async fn list_comparisons(
        &self,
        s: &Scope,
        p: PageRequest,
    ) -> Result<ComparisonPage<ComparisonSummary>> {
        let s = s.clone();
        self.comparison_effect(move |store| store.list(&s, p)).await
    }
    pub async fn read_research_package(&self, s: &Scope, id: &str) -> Result<ResearchPackage> {
        let (s, id) = (s.clone(), id.to_owned());
        self.comparison_effect(move |store| store.package(&s, &id))
            .await
    }
    pub async fn comparison_rows(
        &self,
        s: &Scope,
        id: &str,
        p: PageRequest,
    ) -> Result<ComparisonPage<ComparisonRow>> {
        let (s, id) = (s.clone(), id.to_owned());
        self.comparison_effect(move |store| store.rows(&s, &id, p))
            .await
    }
    pub async fn comparison_sources(
        &self,
        s: &Scope,
        id: &str,
        p: PageRequest,
    ) -> Result<ComparisonPage<ComparisonSource>> {
        let (s, id) = (s.clone(), id.to_owned());
        self.comparison_effect(move |store| store.sources(&s, &id, p))
            .await
    }
    pub async fn research_package_entries(
        &self,
        s: &Scope,
        id: &str,
        p: PageRequest,
    ) -> Result<ComparisonPage<ResearchPackageEntry>> {
        let (s, id) = (s.clone(), id.to_owned());
        self.comparison_effect(move |store| store.package_entries(&s, &id, p))
            .await
    }
}
fn eligible_facts(o: &OfferedOperation) -> bool {
    o.operation == Operation::Facts
        && o.identity.plugin_id == "sec-edgar"
        && o.identity.plugin_version == "0.3.0"
}
fn choose(
    offering: &Offering,
    operation: Operation,
    chosen: Option<&str>,
) -> Result<ProviderIdentity> {
    let eligible: Vec<_> = offering
        .operations()
        .filter(|o| {
            o.operation == operation && (operation != Operation::Facts || eligible_facts(o))
        })
        .filter(|o| chosen.is_none_or(|id| id == o.identity.instance_id))
        .collect();
    match eligible.as_slice() {
        [one] => Ok(one.identity.clone()),
        [] => Err(error(
            ErrorKind::Unavailable,
            "No compatible comparison provider is available",
        )),
        _ => Err(error(
            ErrorKind::AmbiguousProvider,
            "Choose a provider for each comparison capability",
        )),
    }
}
