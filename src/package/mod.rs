//! package identity, version and target. what the manifest describes.

mod artifact;
mod identity;
mod target;
mod version;

pub use artifact::{Artifact, UnknownArtifact};
pub use identity::{IdError, PackageId};
pub use target::{Arch, Libc, Os, Target, TargetError};
pub use version::{Identifier, Version, VersionError};
