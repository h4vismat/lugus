//! Provider-independent financial ingestion and local evidence storage.
pub mod capabilities;
pub mod domain;
pub mod error;

pub mod plugin;

pub mod application;
pub mod filings;
pub mod fundamentals;
pub mod storage;

pub mod historical_prices;
pub mod market_data;

pub mod resolution;

pub mod selection;

pub mod instruments;
