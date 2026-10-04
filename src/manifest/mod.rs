//! `kago.toml` parsing, `when` selectors, normalize, validate and schema migration.

mod kagofile;
mod parse;
mod path;
mod policy;
mod resource;
mod schema;
mod selector;
mod validate;

use std::collections::BTreeMap;
use std::path::Path;

pub(crate) use path::valid_segment;
pub use path::{PathError, Root, SemanticPath};
pub use policy::{
    AppKind, Downgrade, Elevation, InstallPolicy, LicenseFormat, ScopeChoice, ScopeConflict,
    Upgrade,
};
pub use resource::{
    Component, File, Hook, HookPhase, HookPrivilege, Icon, Ownership, PathEntry, PathKind,
    PathSource, Requirement, RequirementKind, RequirementPolicy, ResourceScope, Service,
    ServiceRestart, ServiceStart, Shortcut, ShortcutLocation,
};
pub use selector::{Context, Selector};
pub use validate::SCHEMA;

use crate::diagnostic::{self, Diagnostic, Diagnostics};
use crate::package::{Artifact, Os, PackageId, Version};

/// default manifest file name.
pub const FILE_NAME: &str = "kago.toml";

/// a checked `kago.toml`. every reference inside points at something that
/// exists; which variant applies to a target is decided later by `install`.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub package: Package,
    pub application: Application,
    pub install: InstallPolicy,
    pub license: Option<License>,
    pub components: Vec<Component>,
    pub files: Vec<File>,
    pub paths: Vec<PathEntry>,
    pub shortcuts: Vec<Shortcut>,
    pub services: Vec<Service>,
    pub hooks: Vec<Hook>,
    pub requirements: Vec<Requirement>,
    pub update: Option<Update>,
    /// `targets.<os>.artifacts`, only for listed OSes.
    pub artifacts: BTreeMap<Os, Vec<Artifact>>,
    /// backend settings like `[targets.windows.msi]`, read by that backend.
    pub target_settings: BTreeMap<Os, toml::Table>,
    /// `[distribution.*]`, read by the homebrew and winget generators.
    pub distribution: Option<toml::Table>,
    /// `[ui]`, read by the gui.
    pub ui: Option<toml::Table>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub id: PackageId,
    pub name: String,
    pub version: Version,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub homepage: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Application {
    pub kind: AppKind,
    /// logical resource id of the main executable.
    pub entrypoint: Option<String>,
    pub display_name: String,
    pub description: Option<String>,
    pub categories: Vec<String>,
    pub icons: Vec<Icon>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct License {
    /// build-side path, relative to the manifest directory.
    pub file: String,
    pub format: LicenseFormat,
    pub require_acceptance: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub enabled: bool,
    pub channel: String,
    pub repository: Option<String>,
}

impl Manifest {
    /// checks `source` as the manifest named `file`. returns the manifest
    /// when nothing is an error, plus every diagnostic including warnings.
    #[must_use]
    pub fn parse(file: &str, source: &str) -> (Option<Self>, Diagnostics) {
        match parse::parse(file, source) {
            Ok(raw) => validate::validate(raw, file, source),
            Err(d) => (None, Diagnostics(vec![d])),
        }
    }

    /// reads and checks the manifest at `path`.
    #[must_use]
    pub fn load(path: &Path) -> (Option<Self>, Diagnostics) {
        let file = path.display().to_string();
        match std::fs::read_to_string(path) {
            Ok(source) => Self::parse(&file, &source),
            Err(e) => (
                None,
                Diagnostics(vec![Diagnostic::error(
                    diagnostic::MANIFEST_SYNTAX,
                    format!("cannot read {file}: {e}"),
                )]),
            ),
        }
    }

    #[must_use]
    pub fn component(&self, id: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.id == id)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::diagnostic::Code;

    pub(crate) const AUREA: &str = r#"
schema = 1

[package]
id = "dev.eira.aurea"
name = "Aurea"
version = "1.4.0"
publisher = "Eira"
description = "Aurea desktop application"

[application]
kind = "desktop"
entrypoint = "main-executable"

[install]
scope = "choose"

[license]
file = "LICENSE.txt"
require_acceptance = true

[[components]]
id = "core"
name = "Aurea"
required = true

[[components]]
id = "cli"
name = "Command-line tools"
depends = ["core"]

[[files]]
logical_id = "main-executable"
component = "core"
source = "dist/windows-x86_64/aurea.exe"
target = "{app}/bin/aurea.exe"
when.os = "windows"
when.arch = "x86_64"

[[files]]
logical_id = "main-executable"
component = "core"
source = "dist/linux-x86_64/aurea"
target = "{app}/bin/aurea"
when.os = "linux"
when.arch = "x86_64"

[[files]]
logical_id = "main-executable"
component = "core"
source = "dist/macos-aarch64/aurea"
target = "{app}/Contents/MacOS/Aurea"
when.os = "macos"
when.arch = "aarch64"

[[files]]
component = "core"
source = "themes"
target = "{app}/themes"

[[paths]]
resource = "main-executable"
kind = "executable-parent"
component = "cli"
when.os = ["windows", "linux"]

[[shortcuts]]
name = "Aurea"
target = "main-executable"

[targets.windows]
artifacts = ["setup", "msi"]

[targets.linux]
artifacts = ["setup", "appimage"]
"#;

    fn check(src: &str) -> (Option<Manifest>, Vec<Code>) {
        let (m, d) = Manifest::parse("kago.toml", src);
        (m, d.iter().map(|d| d.code).collect())
    }

    fn with(extra: &str) -> String {
        format!("{AUREA}\n{extra}")
    }

    #[test]
    fn aurea_example_is_valid() {
        let (m, codes) = check(AUREA);
        assert!(codes.is_empty(), "{codes:?}");
        let m = m.unwrap();
        assert_eq!(m.package.id.as_str(), "dev.eira.aurea");
        assert_eq!(m.install.scope, ScopeChoice::Choose);
        assert_eq!(m.files.len(), 4);
        assert!(m.component("core").unwrap().default);
        assert_eq!(
            m.artifacts[&Os::Windows],
            vec![Artifact::Setup, Artifact::Msi]
        );
        assert_eq!(m.shortcuts[0].location, ShortcutLocation::Applications);
        assert_eq!(m.application.display_name, "Aurea");
    }

    #[test]
    fn schema_is_required_and_checked() {
        let (m, codes) = check(&AUREA.replace("schema = 1", ""));
        assert!(m.is_none());
        assert_eq!(codes, vec![diagnostic::MANIFEST_SCHEMA]);
        let (_, codes) = check(&AUREA.replace("schema = 1", "schema = 2"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_SCHEMA]);
    }

    #[test]
    fn bad_values_are_reported() {
        let (_, codes) = check(&AUREA.replace("dev.eira.aurea", "Aurea"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_VALUE]);
        let (_, codes) = check(&AUREA.replace("\"1.4.0\"", "\"1.4\""));
        assert_eq!(codes, vec![diagnostic::MANIFEST_VALUE]);
        let (_, codes) = check(&AUREA.replace("scope = \"choose\"", "scope = \"everyone\""));
        assert_eq!(codes, vec![diagnostic::MANIFEST_VALUE]);
    }

    #[test]
    fn unsafe_target_paths_are_rejected() {
        let (_, codes) = check(&AUREA.replace("{app}/themes", "{app}/../themes"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_PATH]);
        let (_, codes) = check(&AUREA.replace("{app}/themes", "C:/themes"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_PATH]);
    }

    #[test]
    fn references_must_exist() {
        let (_, codes) =
            check(&AUREA.replace("entrypoint = \"main-executable\"", "entrypoint = \"main\""));
        assert_eq!(codes, vec![diagnostic::MANIFEST_REFERENCE]);
        let (_, codes) = check(&with(
            "[[shortcuts]]\nname = \"x\"\ntarget = \"missing\"\ncomponent = \"docs\"\n",
        ));
        assert_eq!(
            codes,
            vec![
                diagnostic::MANIFEST_REFERENCE,
                diagnostic::MANIFEST_REFERENCE
            ]
        );
        let (_, codes) = check(&AUREA.replace("depends = [\"core\"]", "depends = [\"gui\"]"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_REFERENCE]);
    }

    #[test]
    fn duplicates_and_bad_artifacts() {
        let (_, codes) = check(&with("[[components]]\nid = \"core\"\nname = \"again\"\n"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_DUPLICATE]);
        let (_, codes) = check(&AUREA.replace("[\"setup\", \"msi\"]", "[\"setup\", \"dmg\"]"));
        assert_eq!(codes, vec![diagnostic::TARGET_CAPABILITY]);
    }

    #[test]
    fn paths_need_one_source() {
        let (_, codes) = check(&with(
            "[[paths]]\nresource = \"main-executable\"\ndirectory = \"{app}/bin\"\n",
        ));
        assert_eq!(codes, vec![diagnostic::MANIFEST_VALUE]);
    }

    #[test]
    fn errors_point_at_the_value() {
        let src = AUREA.replace("target = \"{app}/themes\"", "target = \"{nope}/themes\"");
        let (_, d) = Manifest::parse("kago.toml", &src);
        let at = d.iter().next().unwrap().location.clone().unwrap();
        let line = src.lines().nth(at.line as usize - 1).unwrap();
        assert!(line.starts_with("target = \"{nope}"), "{line}");
        assert_eq!(at.column, 10);
    }
}
