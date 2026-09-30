//! Deterministic annual financial comparisons over explicitly captured evidence.
mod annual;
mod types;
pub use annual::select_annual;
pub use types::*;
mod calculate;
mod decimal;
pub use calculate::calculate_annual;
