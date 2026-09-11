//! Model-neutral disposable runtime creation and panic isolation.
use super::default_conversation_run_limits;
use crate::{AppError, ErrorKind, Result};
use lugus_agent::{AgentRuntime, RunLimits};
use std::{
    future::{Future, poll_fn},
    panic::{AssertUnwindSafe, catch_unwind},
    task::Poll,
};

#[async_trait::async_trait]
pub trait RuntimeFactory: Send + Sync {
    /// Freeze mutable runtime selection for one admitted message, including preparation.
    /// Immutable factories can retain the default and are reused directly.
    fn snapshot(&self) -> Option<std::sync::Arc<dyn RuntimeFactory>> {
        None
    }

    /// Optional deadline for a selected profile, applied to the whole message.
    fn run_timeout(&self) -> Option<std::time::Duration> {
        None
    }

    /// A new session for one accepted turn. The host enforces deadline and close.
    /// Creation must be cancellation-safe: until returned, the factory owns its child cleanup.
    async fn create(&self) -> Result<Box<dyn AgentRuntime>>;
}
#[derive(Debug, Clone)]
pub struct ConversationOptions {
    pub run_limits: RunLimits,
    pub allow_web_search: bool,
}
impl Default for ConversationOptions {
    fn default() -> Self {
        Self {
            run_limits: default_conversation_run_limits(),
            allow_web_search: false,
        }
    }
}
pub(super) fn failure(kind: ErrorKind, message: &'static str) -> AppError {
    AppError::new(kind, message, false)
}
pub(super) async fn isolate<F: Future>(future: F) -> Result<F::Output> {
    let mut future = std::pin::pin!(future);
    poll_fn(
        |cx| match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(value)) => Poll::Ready(Ok(value)),
            Err(_) => Poll::Ready(Err(failure(
                ErrorKind::Unavailable,
                "runtime task panicked",
            ))),
        },
    )
    .await
}
pub(super) async fn cancelled(cancel: &mut tokio::sync::watch::Receiver<bool>) {
    while !*cancel.borrow_and_update() {
        if cancel.changed().await.is_err() {
            break;
        }
    }
}
