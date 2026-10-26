//! the Kagofile: kago's manifest language, lowered into the raw manifest
//! the validator checks.

mod action;
mod lower;
mod tree;
mod version;

pub use lower::lower;
pub use tree::parse;
