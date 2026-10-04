//! the Kagofile: parsing, `when` selectors, sources and validation.

mod kagofile;
mod path;
mod policy;
mod resource;
mod schema;
mod selector;
mod source;
mod validate;

use std::collections::BTreeMap;
use std::path::Path;

pub(crate) use path::valid_segment;
pub use path::{PathError, Root, SemanticPath};
pub use policy::{
    AppKind, Downgrade, Elevation, InstallPolicy, LicenseFormat, ScopeChoice, ScopeConflict,
    SetupLook, Upgrade,
};
pub use resource::{
    Component, File, Hook, HookPhase, HookPrivilege, Icon, Ownership, PathEntry, PathKind,
    PathSource, Requirement, RequirementKind, RequirementPolicy, ResourceScope, Service,
    ServiceRestart, ServiceStart, Shortcut, ShortcutLocation, SteamVrDriver,
};
pub use selector::{Context, Selector};
pub use source::{Source, SourceKind, Unpack};
pub use validate::SCHEMA;

use crate::diagnostic::{self, Diagnostic, Diagnostics};
use crate::package::{Artifact, Os, PackageId, Version};

/// manifest file name.
pub const FILE_NAME: &str = "Kagofile";

/// a checked Kagofile. every reference inside points at something that
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
    pub steamvr_drivers: Vec<SteamVrDriver>,
    pub shortcuts: Vec<Shortcut>,
    pub services: Vec<Service>,
    pub hooks: Vec<Hook>,
    pub requirements: Vec<Requirement>,
    pub update: Option<Update>,
    /// `target <os> <artifact>...`, only for listed OSes.
    pub artifacts: BTreeMap<Os, Vec<Artifact>>,
    /// where `name:path` file sources come from.
    pub sources: Vec<Source>,
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
    /// checks Kagofile text named `file`, with paths relative to the
    /// current directory. returns the manifest when nothing is an error,
    /// plus every diagnostic including warnings.
    #[must_use]
    pub fn parse(file: &str, source: &str) -> (Option<Self>, Diagnostics) {
        Self::parse_in(file, source, Path::new("."))
    }

    /// as `parse`, with `version from` and relative paths under `base`.
    #[must_use]
    pub fn parse_in(file: &str, source: &str, base: &Path) -> (Option<Self>, Diagnostics) {
        let nodes = match kagofile::parse(file, source) {
            Ok(nodes) => nodes,
            Err(d) => return (None, Diagnostics(vec![d])),
        };
        let (lowered, mut diagnostics) = kagofile::lower(file, source, base, &nodes);
        let Some(lowered) = lowered else {
            return (None, diagnostics);
        };
        let (manifest, more) = validate::validate(lowered.raw, file, source);
        diagnostics.extend(more.0);
        let manifest = manifest.map(|mut m| {
            m.sources = lowered.sources;
            m
        });
        let manifest = if diagnostics.has_errors() {
            None
        } else {
            manifest
        };
        (manifest, diagnostics)
    }

    /// reads and checks the Kagofile at `path`.
    #[must_use]
    pub fn load(path: &Path) -> (Option<Self>, Diagnostics) {
        let file = path.display().to_string();
        match std::fs::read_to_string(path) {
            Ok(source) => Self::parse_in(&file, &source, path.parent().unwrap_or(Path::new("."))),
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
    pub fn source(&self, name: &str) -> Option<&Source> {
        self.sources.iter().find(|s| s.name == name)
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

    pub(crate) const AUREA: &str = r#"# Aurea test package
package dev.eira.aurea
  name Aurea
  version 1.4.0
  publisher Eira
  description Aurea desktop application
  scope choose

license LICENSE.txt accept

component core Aurea required
  app main-executable
    to {app}/bin
    windows x86_64 dist/windows-x86_64/aurea.exe
    linux x86_64 dist/linux-x86_64/aurea
    macos aarch64 dist/macos-aarch64/aurea -> {app}/Contents/MacOS/Aurea
  copy themes

component cli "Command-line tools" needs core
  when windows linux
    path

shortcut Aurea

target windows setup msi
target linux setup appimage
"#;

    fn check(src: &str) -> (Option<Manifest>, Vec<Code>) {
        let (m, d) = Manifest::parse("Kagofile", src);
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
        assert_eq!(
            m.package.description.as_deref(),
            Some("Aurea desktop application")
        );
        assert_eq!(m.install.scope, ScopeChoice::Choose);
        assert_eq!(m.files.len(), 4);
        assert_eq!(m.files[2].target.to_string(), "{app}/Contents/MacOS/Aurea");
        assert_eq!(m.files[3].target.to_string(), "{app}/themes");
        assert!(m.component("core").unwrap().required);
        assert_eq!(m.component("cli").unwrap().depends, ["core"]);
        assert_eq!(
            m.artifacts[&Os::Windows],
            vec![Artifact::Setup, Artifact::Msi]
        );
        assert_eq!(m.shortcuts[0].location, ShortcutLocation::Applications);
        assert_eq!(m.shortcuts[0].target, "main-executable");
        assert_eq!(m.application.entrypoint.as_deref(), Some("main-executable"));
        assert!(m.license.unwrap().require_acceptance);
    }

    #[test]
    fn smallest_kagofile() {
        let (m, codes) =
            check("package dev.x.tool\n  version 1.0.0\napp tool\n  windows tool.exe\nshortcut\n");
        assert!(codes.is_empty(), "{codes:?}");
        let m = m.unwrap();
        assert_eq!(m.package.name, "tool");
        assert_eq!(m.shortcuts[0].name, "tool");
        assert_eq!(m.artifacts[&Os::Windows], vec![Artifact::Setup]);
    }

    #[test]
    fn steamvr_driver_copies_and_registers() {
        let (m, codes) = check(&with(
            "component steamvr \"SteamVR driver\" on
  when windows
    steamvr driver/aira
",
        ));
        assert!(codes.is_empty(), "{codes:?}");
        let m = m.unwrap();
        let driver = &m.steamvr_drivers[0];
        assert_eq!(driver.directory.to_string(), "{app}/steamvr/aira");
        assert_eq!(driver.component.as_deref(), Some("steamvr"));
        let copied = m.files.last().unwrap();
        assert_eq!(copied.source, "driver/aira");
        assert_eq!(copied.target, driver.directory);

        let (m, _) = check(&with(
            "steamvr out/aira -> {data}/steamvr/aira
",
        ));
        assert_eq!(
            m.unwrap().steamvr_drivers[0].directory.to_string(),
            "{data}/steamvr/aira"
        );
        let (_, codes) = check(&with(
            "steamvr a b
",
        ));
        assert_eq!(codes, vec![diagnostic::MANIFEST_SYNTAX]);
    }

    #[test]
    fn setup_look_defaults_to_modern() {
        let (m, _) = check(AUREA);
        assert_eq!(m.unwrap().install.look, SetupLook::Modern);
        let (m, codes) = check(&AUREA.replace(
            "scope choose",
            "scope choose
  setup native",
        ));
        assert!(codes.is_empty(), "{codes:?}");
        assert_eq!(m.unwrap().install.look, SetupLook::Native);
        let (_, codes) = check(&AUREA.replace(
            "scope choose",
            "scope choose
  setup fancy",
        ));
        assert_eq!(codes, vec![diagnostic::MANIFEST_VALUE]);
    }

    #[test]
    fn package_and_version_are_required() {
        let (m, codes) = check("app x\n  windows x.exe\n");
        assert!(m.is_none());
        assert_eq!(codes, vec![diagnostic::MANIFEST_VALUE]);
        let (_, codes) = check("package dev.x.y\n  name Y\n");
        assert_eq!(codes, vec![diagnostic::MANIFEST_VALUE]);
    }

    #[test]
    fn bad_values_are_reported() {
        let (_, codes) = check(&AUREA.replace("dev.eira.aurea", "Aurea"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_VALUE]);
        let (_, codes) = check(&AUREA.replace("version 1.4.0", "version 1.4"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_VALUE]);
        let (_, codes) = check(&AUREA.replace("scope choose", "scope everyone"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_VALUE]);
    }

    #[test]
    fn unknown_statements_are_syntax_errors() {
        let (_, codes) = check(&with("install everything\n"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_SYNTAX]);
        let (_, codes) = check(&with("copy a sideways b\n"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_SYNTAX]);
    }

    #[test]
    fn unsafe_target_paths_are_rejected() {
        let (_, codes) = check(&with("copy x -> {app}/../x\n"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_PATH]);
        let (_, codes) = check(&with("copy x -> C:/x\n"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_PATH]);
    }

    #[test]
    fn references_must_exist() {
        let (_, codes) = check(&with("shortcut Other -> missing\n"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_REFERENCE]);
        let (_, codes) = check(&AUREA.replace("needs core", "needs gui"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_REFERENCE]);
        let (_, codes) = check(&with("copy release:x\n"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_REFERENCE]);
    }

    #[test]
    fn duplicates_and_bad_artifacts() {
        let (_, codes) = check(&with("component core again\n"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_DUPLICATE]);
        let (_, codes) =
            check(&AUREA.replace("target windows setup msi", "target windows setup dmg"));
        assert_eq!(codes, vec![diagnostic::TARGET_CAPABILITY]);
        let (_, codes) = check(&with("when beos\n  copy x\n"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_SELECTOR]);
    }

    #[test]
    fn optional_path_gets_its_own_component() {
        let (m, _) = check(
            "package dev.x.tool\n  version 1.0.0\napp tool\n  windows tool.exe\npath optional \"Add tool to PATH\"\n",
        );
        let m = m.unwrap();
        let c = m.component("path-tool").unwrap();
        assert!(!c.default && !c.required);
        assert_eq!(c.name, "Add tool to PATH");
        assert_eq!(m.paths[0].component.as_deref(), Some("path-tool"));
    }

    #[test]
    fn sources_are_declared_and_checked() {
        let (m, codes) = check(&with(
            "source build\n  dir target/release\nsource release\n  fetch https://x/${version}.zip\n  sha256 windows-x86_64 abc\n  unpack zip\ncopy release:docs\n",
        ));
        assert!(codes.is_empty(), "{codes:?}");
        let m = m.unwrap();
        assert_eq!(m.sources.len(), 2);
        assert_eq!(m.source("release").unwrap().unpack, Unpack::Zip);
        assert_eq!(m.files.last().unwrap().source, "release:docs");
        assert_eq!(m.files.last().unwrap().target.to_string(), "{app}/docs");
        let unpinned = with("source web\n  fetch https://x/y.zip\n");
        let (m, d) = Manifest::parse("Kagofile", &unpinned);
        assert!(m.is_some(), "an unpinned fetch only warns");
        assert_eq!(
            d.iter().next().unwrap().severity,
            crate::diagnostic::Severity::Warning
        );
    }

    #[test]
    fn errors_point_at_the_value() {
        let src = AUREA.replace("copy themes", "copy themes -> {nope}/themes");
        let (_, d) = Manifest::parse("Kagofile", &src);
        let at = d.iter().next().unwrap().location.clone().unwrap();
        let line = src.lines().nth(at.line as usize - 1).unwrap();
        assert_eq!(line.trim(), "copy themes -> {nope}/themes");
        assert_eq!(at.column, 18);
    }
}
