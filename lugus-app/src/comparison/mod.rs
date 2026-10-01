//! Durable, agent-independent company research.
mod lease;
mod store;
mod types;
pub use lease::ComparisonLease;
pub use store::ComparisonStore;
pub use types::*;
