//! Immutable canonical filing text and exact source coordinates. No I/O in this module.
mod domain;
mod html;
mod mapping;
pub use domain::*;
pub use html::{HtmlTextExtractor, extract_html};
pub use mapping::*;
