//! package identity, version and target. what the manifest describes.

mod identity;
mod target;
mod version;

pub use identity::{IdError, PackageId};
pub use target::{Arch, Libc, Os, Target, TargetError};
pub use version::{Identifier, Version, VersionError};
