//! `kago.toml` parsing, `when` selectors, normalize, validate and schema migration.

mod parse;
mod path;
mod schema;
mod selector;

pub use path::{PathError, Root, SemanticPath};
pub use selector::{Context, Selector};
