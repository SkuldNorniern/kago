//! package identity, version and target. what the manifest describes.

mod identity;
mod version;

pub use identity::{IdError, PackageId};
pub use version::{Identifier, Version, VersionError};
