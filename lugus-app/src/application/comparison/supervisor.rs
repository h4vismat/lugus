use super::*;
pub(crate) struct ComparisonExecution {
    pub workspace: String,
    pub cancel: watch::Sender<bool>,
    pub done: watch::Receiver<bool>,
}
pub(super) struct ComparisonStop {
    pub cancel: watch::Receiver<bool>,
    pub deadline: lugus_agent::deadline::Deadline,
}
impl ComparisonStop {
    pub fn check(&self) -> Result<()> {
        if *self.cancel.borrow() {
            Err(error(ErrorKind::Cancelled, "Comparison cancelled"))
        } else if self.deadline.expired() {
            Err(error(ErrorKind::Timeout, "Comparison deadline exceeded"))
        } else {
            Ok(())
        }
    }
}
pub(super) async fn run(
    app: Application,
    scope: Scope,
    job: ComparisonJob,
    offering: Offering,
    lease: ComparisonLease,
    cancel: watch::Receiver<bool>,
    done: watch::Sender<bool>,
) {
    let worker = app.clone();
    let sc = scope.clone();
    let j = job.clone();
    let timeout = app.limits().operation_timeout;
    let task = tokio::spawn(async move {
        let stop = ComparisonStop {
            cancel,
            deadline: lugus_agent::deadline::Deadline::after(timeout),
        };
        let p = prepare::prepare_comparison(&worker, &sc, &j, &offering, &stop).await?;
        stop.check()?;
        let id = j.id.clone();
        worker
            .comparison_effect(move |s| s.publish(&sc, &id, &p))
            .await
    });
    let result = task.await.unwrap_or_else(|_| {
        Err(error(
            ErrorKind::Unavailable,
            "Comparison preparation stopped unexpectedly",
        ))
    });
    if let Err(e) = result {
        let sc = scope.clone();
        let id = job.id.clone();
        let state = if e.kind == ErrorKind::Cancelled {
            ComparisonState::Cancelled
        } else {
            ComparisonState::Failed
        };
        let _ = app
            .comparison_effect(move |s| s.finish(&sc, &id, state, Some(e)))
            .await;
    }
    drop(lease);
    if let Ok(mut state) = app.inner.admission.lock() {
        state.comparisons.remove(&job.id);
    }
    done.send_replace(true);
}
