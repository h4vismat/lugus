//! Application-owned durable conversation contracts and deterministic context selection.
pub mod context;
pub mod domain;
mod frozen;
mod limits;

pub use context::{build_context, build_context_with_omitted};
pub use domain::*;
pub use frozen::{DatasetSelectionCoverage, FrozenDataset, FrozenView};
pub use limits::{ConversationLimits, default_conversation_run_limits};

pub mod ownership;
pub mod store;
pub use ownership::{ExecutionEpoch, LocalExecutionLease, RunAttempt};
pub use store::*;

mod workspace;
pub use workspace::workspace_transition;

mod execution;
mod host;
mod journal;
mod runtime;
pub use host::ConversationHost;
pub use runtime::{ConversationOptions, RuntimeFactory};
