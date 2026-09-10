//! Framework-independent application contracts and routing decisions.

pub mod catalog;
pub mod domain;
pub mod error;

pub use catalog::*;
pub use domain::*;
pub use error::*;

pub mod provider;
pub mod worker;
pub use provider::*;
pub use worker::*;

pub mod agent_contract;
