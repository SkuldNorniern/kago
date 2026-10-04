//! the Kagofile: kago's own manifest language. one statement per line,
//! blocks by indentation, lowered into the raw manifest the validator
//! checks.

mod lower;
mod tree;
mod version;

pub use lower::lower;
pub use tree::parse;
