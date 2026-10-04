//! checked manifest items that become installed resources.

use super::path::SemanticPath;
use super::policy::keywords;
use super::selector::Selector;
use crate::diagnostic::Location;

keywords! {
    /// how uninstall and repair treat a file.
    Ownership {
        Owned => "owned",
        Shared => "shared",
        Generated => "generated",
        UserData => "user-data",
        PreserveIfModified => "preserve-if-modified",
        External => "external",
    }
}

keywords! {
    /// which scope a PATH entry or service lands in.
    ResourceScope { User => "user", Machine => "machine", ResolvedInstall => "resolved-install" }
}

keywords! {
    /// `[[paths]] kind` for resource-based PATH entries.
    PathKind { ExecutableParent => "executable-parent" }
}

keywords! {
    /// where a shortcut goes.
    ShortcutLocation { Applications => "applications", Desktop => "desktop" }
}

keywords! {
    ServiceStart { Automatic => "automatic", Manual => "manual" }
}

keywords! {
    ServiceRestart { Never => "never", OnFailure => "on-failure", Always => "always" }
}

keywords! {
    HookPhase {
        PreInstall => "pre-install",
        PostInstall => "post-install",
        PreUninstall => "pre-uninstall",
        PostUninstall => "post-uninstall",
        PreUpdate => "pre-update",
        PostUpdate => "post-update",
    }
}

keywords! {
    HookPrivilege { User => "user", Elevated => "elevated" }
}

keywords! {
    RequirementKind { Os => "os", Runtime => "runtime", SystemPackage => "system-package" }
}

keywords! {
    RequirementPolicy { Require => "require", InstallIfMissing => "install-if-missing" }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub required: bool,
    pub default: bool,
    pub depends: Vec<String>,
}

/// one `[[files]]` entry. variants of the same thing share `logical_id`.
/// `source` is build-side, relative to the manifest directory; a directory
/// source installs its whole tree under `target`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub logical_id: Option<String>,
    pub component: Option<String>,
    pub source: String,
    pub target: SemanticPath,
    pub ownership: Ownership,
    pub when: Selector,
    pub at: Location,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSource {
    /// directory of the resolved variant of a logical resource.
    Resource {
        id: String,
        kind: PathKind,
    },
    Directory(SemanticPath),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEntry {
    pub source: PathSource,
    pub scope: ResourceScope,
    pub component: Option<String>,
    pub when: Selector,
    pub at: Location,
}

/// a SteamVR driver folder to register with SteamVR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamVrDriver {
    pub directory: SemanticPath,
    pub component: Option<String>,
    pub when: Selector,
    pub at: Location,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub name: String,
    /// logical resource id.
    pub target: String,
    pub location: ShortcutLocation,
    pub arguments: Vec<String>,
    pub component: Option<String>,
    pub when: Selector,
    pub at: Location,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub id: String,
    pub name: Option<String>,
    /// logical resource id.
    pub command: String,
    pub arguments: Vec<String>,
    pub scope: ResourceScope,
    pub start: ServiceStart,
    pub restart: ServiceRestart,
    pub component: Option<String>,
    pub when: Selector,
    pub at: Location,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    pub phase: HookPhase,
    /// logical resource id.
    pub command: String,
    pub arguments: Vec<String>,
    pub privilege: HookPrivilege,
    pub when: Selector,
    pub at: Location,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub id: String,
    pub kind: RequirementKind,
    pub policy: RequirementPolicy,
    pub version: Option<String>,
    pub minimum: Option<String>,
    pub when: Selector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    /// build-side path, relative to the manifest directory.
    pub source: String,
    pub size: Option<u32>,
    pub when: Selector,
}
