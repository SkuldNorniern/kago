//! the manifest as written, before checking. the Kagofile lowers into this;
//! strings stay strings and `validate` turns them into checked types. spans
//! point into the Kagofile.

use serde_spanned::Spanned;

#[derive(Debug)]
pub struct RawManifest {
    pub schema: Option<Spanned<i64>>,
    pub package: RawPackage,
    pub application: Option<RawApplication>,
    pub install: Option<RawInstall>,
    pub license: Option<RawLicense>,
    pub components: Vec<RawComponent>,
    pub files: Vec<RawFile>,
    pub paths: Vec<Spanned<RawPath>>,
    pub steamvr_drivers: Vec<RawSteamVrDriver>,
    pub shortcuts: Vec<RawShortcut>,
    pub services: Vec<RawService>,
    pub hooks: Vec<RawHook>,
    pub requirements: Vec<RawRequirement>,
    pub update: Option<RawUpdate>,
    pub targets: Option<RawTargets>,
}

#[derive(Debug)]
pub struct RawPackage {
    pub id: Spanned<String>,
    pub name: String,
    pub version: Spanned<String>,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub homepage: Option<String>,
}

#[derive(Debug)]
pub struct RawApplication {
    pub kind: Option<Spanned<String>>,
    pub entrypoint: Option<Spanned<String>>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub categories: Vec<String>,
    pub icons: Vec<RawIcon>,
}

#[derive(Debug)]
pub struct RawIcon {
    pub source: Spanned<String>,
    pub size: Option<u32>,
    pub when: Option<RawWhen>,
}

#[derive(Debug)]
pub struct RawInstall {
    pub scope: Option<Spanned<String>>,
    pub scope_conflict: Option<Spanned<String>>,
    pub elevation: Option<Spanned<String>>,
    pub upgrade: Option<Spanned<String>>,
    pub downgrade: Option<Spanned<String>>,
    pub repair: Option<bool>,
    pub rollback: Option<bool>,
    pub look: Option<Spanned<String>>,
    pub folder: Option<Spanned<String>>,
}

#[derive(Debug)]
pub struct RawLicense {
    pub file: Spanned<String>,
    pub format: Option<Spanned<String>>,
    pub require_acceptance: Option<bool>,
}

#[derive(Debug)]
pub struct RawComponent {
    pub id: Spanned<String>,
    pub name: String,
    pub description: Option<String>,
    pub required: Option<bool>,
    pub default: Option<bool>,
    pub depends: Vec<Spanned<String>>,
    pub detect: Option<super::Detect>,
}

#[derive(Debug)]
pub struct RawFile {
    pub logical_id: Option<Spanned<String>>,
    pub component: Option<Spanned<String>>,
    pub source: Spanned<String>,
    pub target: Spanned<String>,
    pub ownership: Option<Spanned<String>>,
    pub when: Option<RawWhen>,
}

#[derive(Debug)]
pub struct RawPath {
    pub resource: Option<Spanned<String>>,
    pub directory: Option<Spanned<String>>,
    pub kind: Option<Spanned<String>>,
    pub scope: Option<Spanned<String>>,
    pub component: Option<Spanned<String>>,
    pub when: Option<RawWhen>,
}

#[derive(Debug)]
pub struct RawSteamVrDriver {
    /// installed folder holding `driver.vrdrivermanifest`.
    pub directory: Spanned<String>,
    pub component: Option<Spanned<String>>,
    pub when: Option<RawWhen>,
}

#[derive(Debug)]
pub struct RawShortcut {
    pub name: String,
    pub target: Spanned<String>,
    pub location: Option<Spanned<String>>,
    pub arguments: Vec<String>,
    pub component: Option<Spanned<String>>,
    pub when: Option<RawWhen>,
}

#[derive(Debug)]
pub struct RawService {
    pub id: Spanned<String>,
    pub name: Option<String>,
    pub command: Spanned<String>,
    pub arguments: Vec<String>,
    pub scope: Option<Spanned<String>>,
    pub start: Option<Spanned<String>>,
    pub restart: Option<Spanned<String>>,
    pub component: Option<Spanned<String>>,
    pub when: Option<RawWhen>,
}

#[derive(Debug)]
pub struct RawHook {
    pub phase: Spanned<String>,
    pub command: Spanned<String>,
    pub arguments: Vec<String>,
    pub privilege: Option<Spanned<String>>,
    pub when: Option<RawWhen>,
}

#[derive(Debug)]
pub struct RawRequirement {
    pub id: Spanned<String>,
    pub kind: Spanned<String>,
    pub policy: Option<Spanned<String>>,
    pub version: Option<String>,
    pub minimum: Option<String>,
    pub detect: Option<super::Detect>,
    pub when: Option<RawWhen>,
}

#[derive(Debug)]
pub struct RawUpdate {
    /// where the update feed is: a `latest.json` written by `kago build`.
    pub feed: Spanned<String>,
}

#[derive(Debug)]
pub struct RawTargets {
    pub windows: Option<RawTarget>,
    pub macos: Option<RawTarget>,
    pub linux: Option<RawTarget>,
}

/// artifacts to build for one OS.
#[derive(Debug)]
pub struct RawTarget {
    pub artifacts: Vec<Spanned<String>>,
}

/// `when windows x86_64`, sorted into keys.
#[derive(Debug, Default)]
pub struct RawWhen {
    pub os: Option<Spanned<Vec<String>>>,
    pub arch: Option<Spanned<Vec<String>>>,
    pub libc: Option<Spanned<Vec<String>>>,
    pub format: Option<Spanned<Vec<String>>>,
    pub profile: Option<Spanned<Vec<String>>>,
    pub channel: Option<Spanned<Vec<String>>>,
}
