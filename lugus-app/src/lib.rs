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

pub mod references;
pub mod store;
pub use references::*;
pub use store::*;

pub mod agent_contract;

mod references_input;

pub mod application;
pub use application::*;

pub mod agent;
pub use agent::*;
pub mod config;
pub use config::*;

pub mod bindings;
pub use bindings::*;

pub mod conversations;

pub mod passages;
pub mod research;

pub mod portfolio;
