//! the Kagofile: parsing, `when` selectors, sources and validation.

mod brand;
mod change;
mod check;
mod firewall;
mod input;
mod kagofile;
mod path;
mod policy;
mod prerequisite;
mod resource;
mod schema;
mod selector;
mod sign;
mod source;
mod validate;

use std::collections::BTreeMap;
use std::path::Path;

pub use brand::{Brand, parse_color};
pub use change::{Change, Edit, Place, Value};
pub use check::{Check, Detect};
pub use firewall::FirewallRule;
pub use input::{Field, FieldKind, InputPage, Problem};
pub(crate) use path::valid_segment;
pub use path::{PathError, Root, SemanticPath};
pub use policy::{
    AppKind, Downgrade, Elevation, FolderChoice, InstallPolicy, LicenseFormat, ScopeChoice,
    ScopeConflict, SetupLook, Upgrade,
};
pub use prerequisite::Prerequisite;
pub use resource::{
    Command, Component, Expect, File, Icon, Ownership, PathEntry, PathKind, PathSource,
    Requirement, RequirementKind, RequirementPolicy, ResourceScope, Run, RunAt, Service,
    ServiceRestart, ServiceStart, Shortcut, ShortcutLocation,
};
pub use selector::{Context, Selector};
pub use sign::{Setting, Sign, SignKey, TIMESTAMP};
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
    /// files the build must find.
    pub expects: Vec<Expect>,
    pub shortcuts: Vec<Shortcut>,
    pub services: Vec<Service>,
    /// programs started at install or uninstall.
    pub runs: Vec<Run>,
    /// what `kago build` runs first to make the files, in order.
    pub build: Vec<Command>,
    /// setup pages of the package's own, with fields to fill in.
    pub inputs: Vec<InputPage>,
    pub requirements: Vec<Requirement>,
    pub update: Option<Update>,
    /// `target <os> <artifact>...`, only for listed OSes.
    pub artifacts: BTreeMap<Os, Vec<Artifact>>,
    /// where `name:path` file sources come from.
    pub sources: Vec<Source>,
    /// how the setup looks beyond the defaults.
    pub brand: Brand,
    /// how `kago build` signs; unsigned without it.
    pub sign: Option<Sign>,
    /// other vendors' installers to run first when their check fails.
    pub prerequisites: Vec<Prerequisite>,
    /// programs Windows Firewall lets accept connections.
    pub firewall: Vec<FirewallRule>,
    /// values set in other programs' settings, the registry or the
    /// environment.
    pub changes: Vec<Change>,
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
    /// URL of the `latest.json` that `kago build` writes next to the
    /// setup, wherever the releases are published.
    pub feed: String,
}

impl Update {
    /// the feed of a GitHub repository's newest release: GitHub redirects
    /// `releases/latest/download/<file>` to that release's file.
    #[must_use]
    pub fn github(repository: &str) -> Self {
        Self {
            feed: format!("https://github.com/{repository}/releases/latest/download/latest.json"),
        }
    }
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
        // Windows editors and PowerShell often start the file with a BOM.
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
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
            m.brand = lowered.brand;
            m.sign = lowered.sign;
            m.prerequisites = lowered.prerequisites;
            m.firewall = lowered.firewall;
            m.changes = lowered.changes;
            m.expects = lowered.expects;
            m.runs = lowered.runs;
            m.build = lowered.build;
            m.inputs = lowered.inputs;
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
    fn steamvr_comes_from_the_library() {
        let (m, codes) = check(&with(
            "component steamvr \"SteamVR driver\" on
  when windows
    steamvr driver/aira
",
        ));
        assert!(codes.is_empty(), "{codes:?}");
        let m = m.unwrap();
        let copied = m.files.last().unwrap();
        assert_eq!(copied.source, "driver/aira");
        assert_eq!(copied.target.to_string(), "{app}/steamvr/aira");
        assert_eq!(copied.component.as_deref(), Some("steamvr"));
        assert_eq!(m.expects[0].source, "driver/aira/driver.vrdrivermanifest");
        let add = m.changes.iter().find(|c| c.edit == Edit::Add).unwrap();
        assert_eq!(
            add.place,
            Place::Json {
                file: "%LOCALAPPDATA%/openvr/openvrpaths.vrpath".into(),
                key: vec!["external_drivers".into()],
            }
        );
        assert_eq!(add.value, Value::text("{app}/steamvr/aira"));
        assert_eq!(add.component.as_deref(), Some("steamvr"));

        let (m, _) = check(&with("steamvr out/aira to={data}/steamvr/aira\n"));
        assert_eq!(
            m.unwrap().files.last().unwrap().target.to_string(),
            "{data}/steamvr/aira"
        );
        let (_, codes) = check(&with("steamvr a b c\n"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_SYNTAX]);
    }

    #[test]
    fn actions_are_written_and_used() {
        let dir = std::env::temp_dir().join(format!("kago-use-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("shared.kago"),
            "action settings file\n  json {config}/$file\n    set theme dark\n",
        )
        .unwrap();
        let src = format!(
            "{AUREA}use shared.kago\naction twice a\n  copy $a\n  settings $a.json\ntwice docs\n"
        );
        let (m, d) = Manifest::parse_in("Kagofile", &src, &dir);
        let codes: Vec<_> = d.iter().map(|d| d.code).collect();
        assert!(codes.is_empty(), "{codes:?}");
        let m = m.unwrap();
        assert_eq!(m.files.last().unwrap().source, "docs");
        assert_eq!(m.changes[0].place.to_string(), "{config}/docs.json theme");

        for (extra, code) in [
            ("action copy x\n  copy $x\n", diagnostic::MANIFEST_VALUE),
            (
                "action a\n  copy x\naction a\n  copy y\n",
                diagnostic::MANIFEST_DUPLICATE,
            ),
            ("action a\n  a\na\n", diagnostic::MANIFEST_SYNTAX),
            ("use missing.kago\n", diagnostic::MANIFEST_REFERENCE),
            ("nope 1\n", diagnostic::MANIFEST_SYNTAX),
        ] {
            let (_, d) = Manifest::parse_in("Kagofile", &format!("{AUREA}{extra}"), &dir);
            assert!(
                d.iter().any(|d| d.code == code),
                "{extra}: {:?}",
                d.iter().map(|d| &d.message).collect::<Vec<_>>()
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pages_hold_fields() {
        let (m, codes) = check(&with(
            "page server \"Server settings\"
  about Where trackers send their data.
  text host \"Address\" default=localhost required
  text key \"API key\" secret
  number port \"Port\" default=9000 min=1 max=65535
  choice mode \"Mode\" local cloud default=cloud
  check telemetry \"Send usage data\" on
",
        ));
        assert!(codes.is_empty(), "{codes:?}");
        let page = &m.unwrap().inputs[0];
        assert_eq!(page.title, "Server settings");
        assert_eq!(
            page.about.as_deref(),
            Some("Where trackers send their data.")
        );
        let f = &page.fields;
        assert!(f[0].required && f[0].default == "localhost");
        assert!(f[1].secret());
        assert_eq!(
            f[2].kind,
            FieldKind::Number {
                min: Some(1),
                max: Some(65535)
            }
        );
        assert_eq!(f[3].default, "cloud");
        assert_eq!(f[4].default, "true");
        for bad in [
            "page p\n",
            "page p\n  text a\n  text a\n",
            "page p\n  number n default=x\n",
            "page p\n  choice c\n",
            "page p\n  slider s\n",
            "page p\n  text t bold\n",
            "page p\n  number n default=0 min=1\n",
        ] {
            let (m, codes) = check(&with(bad));
            assert!(m.is_none() && !codes.is_empty(), "{bad}");
        }
    }

    #[test]
    fn build_lines_run_in_order() {
        let (m, codes) = check(&with(
            "build python -m PyInstaller app.spec\nbuild cargo build --release\n",
        ));
        assert!(codes.is_empty(), "{codes:?}");
        let build = m.unwrap().build;
        assert_eq!(build[0].program, "python");
        assert_eq!(build[0].arguments, ["-m", "PyInstaller", "app.spec"]);
        assert_eq!(build[1].program, "cargo");
        let (_, codes) = check(&with("build\n"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_SYNTAX]);
    }

    #[test]
    fn vcredist_comes_from_the_library() {
        let (m, codes) = check(&with("vcredist vc_redist.x64.exe\n"));
        assert!(codes.is_empty(), "{codes:?}");
        let p = &m.unwrap().prerequisites[0];
        assert_eq!(p.name, "Microsoft Visual C++ Runtime");
        assert_eq!(p.file, "vc_redist.x64.exe");
        assert!(p.admin);
        assert_eq!(p.arguments, ["/install", "/quiet", "/norestart"]);
    }

    #[test]
    fn brand_block_reads_every_field() {
        let (m, codes) = check(&with(
            "brand
  text Eira Installer
  accent 6667ab
  logo brand/logo.png
  header brand/header.png
  side brand/side.png
  welcome Thanks for choosing Aurea.
  finish Aurea is ready.
  link \"Read the guide\" https://example.com/guide
",
        ));
        assert!(codes.is_empty(), "{codes:?}");
        let b = m.unwrap().brand;
        assert_eq!(b.text.as_deref(), Some("Eira Installer"));
        assert_eq!(b.accent, Some([0x66, 0x67, 0xab]));
        assert_eq!(b.side.as_deref(), Some("brand/side.png"));
        assert_eq!(
            b.link,
            Some(("Read the guide".into(), "https://example.com/guide".into()))
        );
        let (_, codes) = check(&with(
            "brand
  accent purple
",
        ));
        assert_eq!(codes, vec![diagnostic::MANIFEST_VALUE]);
        let (_, codes) = check(&with(
            "brand
  sparkle on
",
        ));
        assert_eq!(codes, vec![diagnostic::MANIFEST_SYNTAX]);
    }

    #[test]
    fn service_and_run_lines() {
        let (m, codes) = check(&with(
            "service main-executable --tray
  name \"Aurea tray\"
run main-executable --setup
  undo main-executable --unsetup
run {app}/tools/clean.exe --all
  at uninstall
  required
",
        ));
        assert!(codes.is_empty(), "{codes:?}");
        let m = m.unwrap();
        assert_eq!(m.services[0].command, "main-executable");
        assert_eq!(m.services[0].arguments, ["--tray"]);
        assert_eq!(m.services[0].name.as_deref(), Some("Aurea tray"));
        assert_eq!(m.runs[0].phase, RunAt::Install);
        assert_eq!(m.runs[0].undo.as_ref().unwrap().arguments, ["--unsetup"]);
        assert_eq!(m.runs[1].phase, RunAt::Uninstall);
        assert_eq!(m.runs[1].command.program, "{app}/tools/clean.exe");
        assert!(m.runs[1].required && !m.runs[0].required);
        for bad in [
            "run\n",
            "hook post-install main-executable\n",
            "run x\n  at later\n",
            "run x\n  at uninstall\n  undo y\n",
            "run x\n  required\n",
        ] {
            let (m, codes) = check(&with(bad));
            assert!(m.is_none() && !codes.is_empty(), "{bad}");
        }
    }

    #[test]
    fn require_reads_os_minimums_and_software() {
        let (m, codes) = check(&with(
            "require windows 10.0.19041
require steamvr
",
        ));
        assert!(codes.is_empty(), "{codes:?}");
        let m = m.unwrap();
        assert_eq!(m.requirements.len(), 2);
        assert_eq!(m.requirements[0].kind, RequirementKind::Os);
        assert_eq!(m.requirements[0].minimum.as_deref(), Some("10.0.19041"));
        assert_eq!(m.requirements[1].kind, RequirementKind::Runtime);
        let (_, codes) = check(&with(
            "require windows
",
        ));
        assert_eq!(codes, vec![diagnostic::MANIFEST_SYNTAX]);
    }

    #[test]
    fn components_take_about_and_detect() {
        let (m, codes) = check(&with(
            "component steamvr \"SteamVR driver\" on
  about Shows trackers in SteamVR
  detect steamvr
  copy driver
",
        ));
        assert!(codes.is_empty(), "{codes:?}");
        let m = m.unwrap();
        let c = m.component("steamvr").unwrap();
        assert_eq!(c.description.as_deref(), Some("Shows trackers in SteamVR"));
        let detect = c.detect.clone().unwrap();
        assert_eq!(detect.name, "SteamVR", "from the library");
        assert!(matches!(detect.check, Check::Json { .. }));
        assert_eq!(
            m.files.last().unwrap().component.as_deref(),
            Some("steamvr")
        );
        let (_, codes) = check(&with(
            "component x
  detect unicorn
",
        ));
        assert_eq!(codes, vec![diagnostic::MANIFEST_REFERENCE]);
    }

    #[test]
    fn checks_are_written_in_the_kagofile() {
        let (m, codes) = check(&with(
            "check python \"Python 3.12\"\n  registry HKLM\\SOFTWARE\\Python\\PythonCore\\3.12\ncheck steamvr \"My SteamVR\" file %ProgramFiles%/Steam/steam.exe\nrequire python\ncomponent vr VR\n  detect steamvr\n  copy driver\n",
        ));
        assert!(codes.is_empty(), "{codes:?}");
        let m = m.unwrap();
        let python = m.requirements.iter().find(|r| r.id == "python").unwrap();
        assert_eq!(python.detect.as_ref().unwrap().name, "Python 3.12");
        let vr = m.component("vr").unwrap().detect.clone().unwrap();
        assert_eq!(vr.name, "My SteamVR", "the Kagofile replaces the library's");
        let (_, codes) = check(&with("check a file x\ncheck a file y\n"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_DUPLICATE]);
        let (_, codes) = check(&with("check a \"A\"\n"));
        assert_eq!(codes, vec![diagnostic::MANIFEST_SYNTAX]);
    }

    #[test]
    fn sign_blocks() {
        let head = "package dev.x.y\n  version 1.0.0\n";
        let parse = |sign: &str| Manifest::parse("Kagofile", &format!("{head}{sign}"));
        let m = parse("sign\n  certificate env KAGO_THUMB\n  programs off\n")
            .0
            .unwrap();
        let sign = m.sign.unwrap();
        assert_eq!(
            sign.key,
            SignKey::Certificate {
                thumbprint: Setting::Env("KAGO_THUMB".into()),
                machine: false,
            }
        );
        assert!(!sign.programs);
        assert_eq!(sign.timestamp, TIMESTAMP);
        let m = parse("sign\n  pfx certs/code.pfx env KAGO_PFX\n")
            .0
            .unwrap();
        assert_eq!(
            m.sign.unwrap().key,
            SignKey::Pfx {
                file: "certs/code.pfx".into(),
                password: Some("KAGO_PFX".into()),
            }
        );
        assert!(
            parse("sign\n  pfx certs/code.pfx hunter2\n").0.is_none(),
            "no passwords in the Kagofile"
        );
        assert!(
            parse("sign\n  timestamp http://t\n").0.is_none(),
            "needs a key"
        );
        assert!(
            parse("sign\n  certificate AB\n  certificate CD\n")
                .0
                .is_none(),
            "one key"
        );
        assert!(parse("").0.unwrap().sign.is_none());
    }

    #[test]
    fn prerequisites_take_presets_and_their_own_checks() {
        let head = "package dev.x.y\n  version 1.0.0\nsource vc\n  dir deps\n";
        let parse = |p: &str| Manifest::parse("Kagofile", &format!("{head}{p}"));
        let m = parse("vcredist vc:vc_redist.x64.exe\n").0.unwrap();
        let vc = &m.prerequisites[0];
        assert_eq!(vc.name, "Microsoft Visual C++ Runtime");
        assert!(vc.admin);
        assert_eq!(vc.arguments, ["/install", "/quiet", "/norestart"]);
        let m = parse(
            "prerequisite tool \"Tool Runtime\"\n  file vc:tool.msi\n  arguments /qn\n  detect file %ProgramFiles%\\Tool\\tool.exe\n",
        )
        .0
        .unwrap();
        let tool = &m.prerequisites[0];
        assert_eq!(
            tool.check,
            Check::File {
                path: "%ProgramFiles%\\Tool\\tool.exe".into()
            }
        );
        assert!(!tool.admin);
        assert!(
            parse("prerequisite tool\n  file vc:t.exe\n").0.is_none(),
            "needs a check"
        );
        assert!(
            parse("prerequisite vcredist\n").0.is_none(),
            "needs its file"
        );
    }

    #[test]
    fn update_from_github_or_a_feed() {
        let head = "package dev.x.y\n  version 1.0.0\n";
        let parse = |u: &str| Manifest::parse("Kagofile", &format!("{head}{u}"));
        let m = parse("update github SkuldNorniern/aira-studio\n")
            .0
            .unwrap();
        assert_eq!(
            m.update.unwrap().feed,
            "https://github.com/SkuldNorniern/aira-studio/releases/latest/download/latest.json"
        );
        let m = parse("update feed https://example.com/aira/latest.json\n")
            .0
            .unwrap();
        assert_eq!(
            m.update.unwrap().feed,
            "https://example.com/aira/latest.json"
        );
        assert!(
            parse("update feed http://example.com/latest.json\n")
                .0
                .is_none(),
            "https only"
        );
        assert!(parse("update github not-a-repo\n").0.is_none());
    }

    #[test]
    fn a_byte_order_mark_is_skipped() {
        let (m, d) = Manifest::parse("Kagofile", "\u{feff}package dev.x.y\n  version 1.0.0\n");
        assert!(m.is_some(), "{d:?}");
    }

    #[test]
    fn folder_choice_defaults_to_choose() {
        let (m, _) = check(AUREA);
        assert_eq!(m.unwrap().install.folder, FolderChoice::Choose);
        let (m, codes) = check(&AUREA.replace(
            "scope choose",
            "scope choose
  folder fixed",
        ));
        assert!(codes.is_empty(), "{codes:?}");
        assert_eq!(m.unwrap().install.folder, FolderChoice::Fixed);
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
    fn changes_read_from_one_line_or_a_block() {
        let (m, codes) = check(&with(
            "json %LOCALAPPDATA%/openvr/openvrpaths.vrpath\n  init version 1\n  add external_drivers {app}/driver\nregistry HKCU\\Software\\Aurea set @ \"1\"\nenv set AUREA_HOME {app}\n",
        ));
        assert!(codes.is_empty(), "{codes:?}");
        let changes = m.unwrap().changes;
        assert_eq!(changes.len(), 4);
        assert_eq!(changes[0].edit, Edit::Init);
        assert_eq!(changes[0].value, Value::word("1", false));
        assert_eq!(
            changes[1].place,
            Place::Json {
                file: "%LOCALAPPDATA%/openvr/openvrpaths.vrpath".into(),
                key: vec!["external_drivers".into()],
            }
        );
        assert_eq!(
            changes[2].place,
            Place::Registry {
                key: r"HKCU\Software\Aurea".into(),
                name: String::new(),
            }
        );
        assert_eq!(changes[2].value, Value::text("1"), "quoted stays text");
        assert_eq!(changes[3].place.to_string(), "%AUREA_HOME%");
    }

    #[test]
    fn bad_changes_are_errors() {
        for line in [
            "json settings.json set a 1",
            "json %X%/../a.json set a 1",
            "json {config}/a.json set a..b 1",
            "registry Software\\X add List 1",
            "env set BAD-NAME 1",
            "env put A 1",
            "json {config}/a.json",
        ] {
            let (m, codes) = check(&with(&format!("{line}\n")));
            assert!(m.is_none() && !codes.is_empty(), "{line}");
        }
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
