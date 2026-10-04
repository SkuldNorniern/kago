//! `kago.toml` as written. strings stay strings here, `validate` turns them
//! into checked types. spans are kept where a later error needs to point.

use serde::Deserialize;
use toml::Spanned;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawManifest {
    pub schema: Option<Spanned<i64>>,
    pub package: RawPackage,
    pub application: Option<RawApplication>,
    pub install: Option<RawInstall>,
    pub license: Option<RawLicense>,
    #[serde(default)]
    pub components: Vec<RawComponent>,
    #[serde(default)]
    pub files: Vec<RawFile>,
    #[serde(default)]
    pub paths: Vec<Spanned<RawPath>>,
    #[serde(default)]
    pub shortcuts: Vec<RawShortcut>,
    #[serde(default)]
    pub services: Vec<RawService>,
    #[serde(default)]
    pub hooks: Vec<RawHook>,
    #[serde(default)]
    pub requirements: Vec<RawRequirement>,
    pub update: Option<RawUpdate>,
    pub targets: Option<RawTargets>,
    pub distribution: Option<toml::Table>,
    pub ui: Option<toml::Table>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPackage {
    pub id: Spanned<String>,
    pub name: String,
    pub version: Spanned<String>,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub homepage: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawApplication {
    pub kind: Option<Spanned<String>>,
    pub entrypoint: Option<Spanned<String>>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub icons: Vec<RawIcon>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawIcon {
    pub source: Spanned<String>,
    pub size: Option<u32>,
    pub when: Option<RawWhen>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawInstall {
    pub scope: Option<Spanned<String>>,
    pub scope_conflict: Option<Spanned<String>>,
    pub elevation: Option<Spanned<String>>,
    pub upgrade: Option<Spanned<String>>,
    pub downgrade: Option<Spanned<String>>,
    pub repair: Option<bool>,
    pub rollback: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawLicense {
    pub file: Spanned<String>,
    pub format: Option<Spanned<String>>,
    pub require_acceptance: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawComponent {
    pub id: Spanned<String>,
    pub name: String,
    pub description: Option<String>,
    pub required: Option<bool>,
    pub default: Option<bool>,
    #[serde(default)]
    pub depends: Vec<Spanned<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawFile {
    pub logical_id: Option<Spanned<String>>,
    pub component: Option<Spanned<String>>,
    pub source: Spanned<String>,
    pub target: Spanned<String>,
    pub ownership: Option<Spanned<String>>,
    pub when: Option<RawWhen>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPath {
    pub resource: Option<Spanned<String>>,
    pub directory: Option<Spanned<String>>,
    pub kind: Option<Spanned<String>>,
    pub scope: Option<Spanned<String>>,
    pub component: Option<Spanned<String>>,
    pub when: Option<RawWhen>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawShortcut {
    pub name: String,
    pub target: Spanned<String>,
    pub location: Option<Spanned<String>>,
    #[serde(default)]
    pub arguments: Vec<String>,
    pub component: Option<Spanned<String>>,
    pub when: Option<RawWhen>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawService {
    pub id: Spanned<String>,
    pub name: Option<String>,
    pub command: Spanned<String>,
    #[serde(default)]
    pub arguments: Vec<String>,
    pub scope: Option<Spanned<String>>,
    pub start: Option<Spanned<String>>,
    pub restart: Option<Spanned<String>>,
    pub component: Option<Spanned<String>>,
    pub when: Option<RawWhen>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawHook {
    pub phase: Spanned<String>,
    pub command: Spanned<String>,
    #[serde(default)]
    pub arguments: Vec<String>,
    pub privilege: Option<Spanned<String>>,
    pub when: Option<RawWhen>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawRequirement {
    pub id: Spanned<String>,
    pub kind: Spanned<String>,
    pub policy: Option<Spanned<String>>,
    pub version: Option<String>,
    pub minimum: Option<String>,
    pub when: Option<RawWhen>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawUpdate {
    pub enabled: Option<bool>,
    pub channel: Option<String>,
    pub repository: Option<Spanned<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawTargets {
    pub windows: Option<RawTarget>,
    pub macos: Option<RawTarget>,
    pub linux: Option<RawTarget>,
}

/// `artifacts` plus backend settings like `[targets.windows.msi]`, kept raw
/// until the backend that reads them exists.
#[derive(Debug, Deserialize)]
pub struct RawTarget {
    #[serde(default)]
    pub artifacts: Vec<Spanned<String>>,
    #[serde(flatten)]
    pub settings: toml::Table,
}

/// `when = { os = "windows", arch = ["x86_64", "aarch64"] }`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawWhen {
    pub os: Option<Spanned<OneOrMany>>,
    pub arch: Option<Spanned<OneOrMany>>,
    pub libc: Option<Spanned<OneOrMany>>,
    pub format: Option<Spanned<OneOrMany>>,
    pub profile: Option<Spanned<OneOrMany>>,
    pub channel: Option<Spanned<OneOrMany>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    #[must_use]
    pub fn values(&self) -> &[String] {
        match self {
            Self::One(one) => std::slice::from_ref(one),
            Self::Many(many) => many,
        }
    }
}
