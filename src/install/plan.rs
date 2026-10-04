use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use super::need::Need;
use super::operation::{Digest, Operation};
use super::{Resolved, Scope};
use crate::diagnostic::{self, Diagnostic, Diagnostics};
use crate::manifest::{Detect, Downgrade, File, Manifest, SemanticPath};
use crate::package::{PackageId, Target, Version};

/// where a file source like `build:app.exe` or `themes` is on disk.
pub trait Sources {
    /// # Errors
    ///
    /// when the source cannot be found or prepared, like a failed fetch.
    fn locate(&self, reference: &str) -> Result<PathBuf, String>;
}

/// plain paths relative to a directory.
impl Sources for Path {
    fn locate(&self, reference: &str) -> Result<PathBuf, String> {
        Ok(self.join(reference))
    }
}

/// everything one install does, in order. same manifest, target, scope,
/// components and payload give the same plan.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstallPlan {
    pub package: PackageId,
    pub name: String,
    pub version: Version,
    pub publisher: Option<String>,
    pub target: Target,
    pub scope: Scope,
    pub components: Vec<String>,
    /// whether this may replace a newer installed version.
    pub downgrade: Downgrade,
    pub operations: Vec<Operation>,
    /// optional components still open to the person installing, with the
    /// operations each brings. empty once [`InstallPlan::select`] ran.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    /// `{app}` somewhere other than the platform's default, chosen at
    /// install. upgrades keep the folder of the install they replace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<PathBuf>,
    /// what the machine must have before anything changes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<Need>,
}

/// an optional component the setup can offer as a tick box.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Choice {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// ticked unless the person unticks it.
    pub default: bool,
    pub depends: Vec<String>,
    /// indexes into `operations` that only this component brings.
    pub operations: Vec<usize>,
    /// bytes its files take once installed.
    #[serde(default)]
    pub size: u64,
    /// what it is for, when the setup should look for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detect: Option<Detect>,
}

impl InstallPlan {
    /// turns a resolution into concrete steps. reads the payload under
    /// the payload through `sources` to expand directories and hash files.
    #[must_use]
    pub fn build<S: Sources + ?Sized>(
        manifest: &Manifest,
        resolved: &Resolved,
        target: Target,
        scope: Scope,
        sources: &S,
    ) -> (Option<Self>, Diagnostics) {
        let mut diagnostics = Diagnostics::default();
        let mut files = Vec::new();
        for f in &resolved.files {
            if let Err(d) = payload(f, sources, &mut files) {
                diagnostics.push(d);
            }
        }
        files.sort_by(|a, b| a.target.cmp(&b.target));

        let mut directories = BTreeSet::new();
        for f in &files {
            let mut dir = f.target.parent();
            while !dir.relative().is_empty() && directories.insert(dir.clone()) {
                dir = dir.parent();
            }
        }

        // the component each operation comes from, in step with `operations`.
        let mut owners: Vec<Option<String>> = vec![None; directories.len()];
        let mut operations: Vec<Operation> = directories
            .into_iter()
            .map(|path| Operation::CreateDirectory { path })
            .collect();
        for f in files {
            owners.push(f.component);
            operations.push(Operation::InstallFile {
                source: f.source,
                target: f.target,
                size: f.size,
                sha256: f.sha256,
                ownership: f.ownership,
            });
        }
        for p in &resolved.paths {
            owners.push(p.component.clone());
            operations.push(Operation::AddPathEntry {
                directory: p.directory.clone(),
                scope: scope.of(p.scope),
            });
        }
        for driver in &resolved.steamvr_drivers {
            if let Err(d) = driver_shipped(driver, &operations) {
                diagnostics.push(d);
            }
            owners.push(driver.component.clone());
            operations.push(Operation::RegisterSteamVrDriver {
                directory: driver.directory.clone(),
            });
        }
        for s in &resolved.shortcuts {
            owners.push(s.shortcut.component.clone());
            operations.push(Operation::CreateShortcut {
                name: s.shortcut.name.clone(),
                target: s.target.clone(),
                location: s.shortcut.location,
                arguments: s.shortcut.arguments.clone(),
            });
        }
        for s in &resolved.services {
            owners.push(s.service.component.clone());
            operations.push(Operation::InstallService {
                id: s.service.id.clone(),
                command: s.command.clone(),
                arguments: s.service.arguments.clone(),
                scope: scope.of(s.service.scope),
            });
        }
        let entrypoint = manifest.application.entrypoint.as_ref().and_then(|id| {
            resolved
                .files
                .iter()
                .find(|f| f.logical_id.as_ref() == Some(id))
                .map(|f| f.target.clone())
        });
        operations.push(Operation::RegisterApplication {
            display_name: manifest.application.display_name.clone(),
            entrypoint,
        });

        if diagnostics.has_errors() {
            return (None, diagnostics);
        }
        let choices = choices(manifest, resolved, &owners, &operations);
        let needs = needs(resolved, &mut diagnostics);
        if diagnostics.has_errors() {
            return (None, diagnostics);
        }
        let plan = Self {
            package: manifest.package.id.clone(),
            name: manifest.package.name.clone(),
            version: manifest.package.version.clone(),
            publisher: manifest.package.publisher.clone(),
            target,
            scope,
            components: resolved.components.iter().map(str::to_owned).collect(),
            downgrade: manifest.install.downgrade,
            operations,
            choices,
            location: None,
            needs,
        };
        (Some(plan), diagnostics)
    }

    /// the plan with only the `chosen` optional components, plus what they
    /// depend on. operations of the others go, and so do folders nothing
    /// is installed into any more.
    #[must_use]
    pub fn select<'a>(&self, chosen: impl IntoIterator<Item = &'a str>) -> Self {
        let mut on: BTreeSet<&str> = BTreeSet::new();
        let mut todo: Vec<&str> = chosen.into_iter().collect();
        while let Some(id) = todo.pop() {
            if let Some(choice) = self.choices.iter().find(|c| c.id == id)
                && on.insert(choice.id.as_str())
            {
                todo.extend(choice.depends.iter().map(String::as_str));
            }
        }
        let dropped: BTreeSet<usize> = self
            .choices
            .iter()
            .filter(|c| !on.contains(c.id.as_str()))
            .flat_map(|c| c.operations.iter().copied())
            .collect();
        let kept: Vec<Operation> = self
            .operations
            .iter()
            .enumerate()
            .filter(|(i, _)| !dropped.contains(i))
            .map(|(_, op)| op.clone())
            .collect();
        let mut needed = BTreeSet::new();
        for op in &kept {
            if let Operation::InstallFile { target, .. } = op {
                let mut dir = target.parent();
                while !dir.relative().is_empty() && needed.insert(dir.clone()) {
                    dir = dir.parent();
                }
            }
        }
        let operations = kept
            .into_iter()
            .filter(|op| match op {
                Operation::CreateDirectory { path } => needed.contains(path),
                _ => true,
            })
            .collect();
        let components = self
            .components
            .iter()
            .filter(|id| {
                self.choices
                    .iter()
                    .all(|c| c.id != **id || on.contains(c.id.as_str()))
            })
            .cloned()
            .collect();
        Self {
            components,
            operations,
            choices: Vec::new(),
            ..self.clone()
        }
    }

    /// the plan installing `{app}` into `dir`.
    #[must_use]
    pub fn at(&self, dir: &Path) -> Self {
        Self {
            location: Some(dir.to_path_buf()),
            ..self.clone()
        }
    }

    /// the plan as the defaults would install it.
    #[must_use]
    pub fn with_defaults(&self) -> Self {
        self.select(
            self.choices
                .iter()
                .filter(|c| c.default)
                .map(|c| c.id.as_str()),
        )
    }

    /// total payload bytes.
    #[must_use]
    pub fn payload_size(&self) -> u64 {
        self.operations
            .iter()
            .map(|op| match op {
                Operation::InstallFile { size, .. } => *size,
                _ => 0,
            })
            .sum()
    }
}

impl fmt::Display for InstallPlan {
    /// dry-run listing.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} {} ({})", self.name, self.version, self.package)?;
        writeln!(f, "target: {}", self.target)?;
        writeln!(f, "scope: {}", self.scope)?;
        writeln!(f, "components: {}", self.components.join(", "))?;
        writeln!(f, "payload: {} bytes", self.payload_size())?;
        writeln!(f)?;
        for op in &self.operations {
            writeln!(f, "  {op}")?;
        }
        Ok(())
    }
}

/// what the resolved requirements need, with a diagnostic for each kago
/// cannot check yet.
fn needs(resolved: &Resolved, diagnostics: &mut Diagnostics) -> Vec<Need> {
    let mut out = Vec::new();
    for requirement in &resolved.requirements {
        match Need::from_requirement(requirement) {
            Ok(need) => out.push(need),
            Err(d) => diagnostics.push(d),
        }
    }
    out
}

/// a SteamVR driver folder must ship its `driver.vrdrivermanifest`, or
/// SteamVR skips it without a word.
fn driver_shipped(
    driver: &crate::manifest::SteamVrDriver,
    operations: &[Operation],
) -> Result<(), Diagnostic> {
    let manifest_file = driver.directory.join("driver.vrdrivermanifest");
    let shipped = operations
        .iter()
        .any(|op| matches!(op, Operation::InstallFile { target, .. } if *target == manifest_file));
    if shipped {
        return Ok(());
    }
    Err(Diagnostic::error(
        diagnostic::INSTALL_SOURCE,
        format!(
            "SteamVR driver `{}` has no `driver.vrdrivermanifest`; point `steamvr` at the driver folder that holds it",
            driver.directory
        ),
    )
    .at(driver.at.clone()))
}

/// the optional components chosen in `resolved`, each with the operations
/// only it brings. `owners` names the component of every operation.
fn choices(
    manifest: &Manifest,
    resolved: &Resolved,
    owners: &[Option<String>],
    all: &[Operation],
) -> Vec<Choice> {
    manifest
        .components
        .iter()
        .filter(|c| !c.required && resolved.components.contains(Some(&c.id)))
        .filter_map(|c| {
            let operations: Vec<usize> = owners
                .iter()
                .enumerate()
                .filter(|(_, owner)| owner.as_deref() == Some(c.id.as_str()))
                .map(|(i, _)| i)
                .collect();
            (!operations.is_empty()).then(|| Choice {
                id: c.id.clone(),
                name: c.name.clone(),
                description: c.description.clone(),
                default: c.default,
                depends: c.depends.clone(),
                size: operations
                    .iter()
                    .map(|i| match &all[*i] {
                        Operation::InstallFile { size, .. } => *size,
                        _ => 0,
                    })
                    .sum(),
                operations,
                detect: c.detect,
            })
        })
        .collect()
}

/// one payload file after directory expansion.
struct Payload {
    component: Option<String>,
    source: PathBuf,
    target: SemanticPath,
    size: u64,
    sha256: Digest,
    ownership: crate::manifest::Ownership,
}

/// `file` as payload entries. a directory source becomes every file under
/// it, sorted, mapped below `file.target`.
fn payload<S: Sources + ?Sized>(
    file: &File,
    sources: &S,
    out: &mut Vec<Payload>,
) -> Result<(), Diagnostic> {
    let error = |message: String| {
        Diagnostic::error(diagnostic::INSTALL_SOURCE, message).at(file.at.clone())
    };
    let source = sources.locate(&file.source).map_err(error)?;
    let meta = fs::metadata(&source)
        .map_err(|e| error(format!("cannot read payload `{}`: {e}", file.source)))?;
    if meta.is_file() {
        out.push(entry(&source, file.target.clone(), file)?);
        return Ok(());
    }
    let mut stack = vec![(source, file.target.clone())];
    while let Some((dir, target)) = stack.pop() {
        let mut children: Vec<_> = fs::read_dir(&dir)
            .and_then(Iterator::collect::<io::Result<Vec<_>>>)
            .map_err(|e| error(format!("cannot list `{}`: {e}", dir.display())))?;
        children.sort_by_key(fs::DirEntry::file_name);
        for child in children {
            let name = child.file_name();
            let Some(name) = name.to_str().filter(|n| crate::manifest::valid_segment(n)) else {
                return Err(error(format!(
                    "payload name `{}` under `{}` is not portable",
                    name.to_string_lossy(),
                    dir.display()
                )));
            };
            let path = child.path();
            let child_target = target.join(name);
            if path.is_dir() {
                stack.push((path, child_target));
            } else {
                out.push(entry(&path, child_target, file)?);
            }
        }
    }
    Ok(())
}

fn entry(source: &Path, target: SemanticPath, file: &File) -> Result<Payload, Diagnostic> {
    let (size, sha256) = hash(source).map_err(|e| {
        Diagnostic::error(
            diagnostic::INSTALL_SOURCE,
            format!("cannot read payload `{}`: {e}", source.display()),
        )
        .at(file.at.clone())
    })?;
    Ok(Payload {
        component: file.component.clone(),
        source: source.to_path_buf(),
        target,
        size,
        sha256,
        ownership: file.ownership,
    })
}

/// size and sha-256 of a file, streamed.
///
/// # Errors
///
/// when the file cannot be read.
pub fn hash(path: &Path) -> io::Result<(u64, Digest)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    let mut size = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((size, Digest(hasher.finalize().into())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::{Selection, resolve};
    use crate::manifest::{Context, tests::AUREA};

    /// temp payload tree matching the AUREA example.
    fn payload_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kago-plan-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dist/windows-x86_64")).unwrap();
        fs::create_dir_all(dir.join("themes/dark")).unwrap();
        fs::write(dir.join("dist/windows-x86_64/aurea.exe"), b"MZ fake").unwrap();
        fs::write(dir.join("themes/light.toml"), b"light").unwrap();
        fs::write(dir.join("themes/dark/main.toml"), b"dark").unwrap();
        dir
    }

    fn plan(dir: &Path) -> (Option<InstallPlan>, Diagnostics) {
        let m = Manifest::parse("Kagofile", AUREA).0.unwrap();
        let target: Target = "windows-x86_64".parse().unwrap();
        let r = resolve(&m, &Context::target(target), Selection::defaults(&m))
            .0
            .unwrap();
        InstallPlan::build(&m, &r, target, Scope::User, dir)
    }

    #[test]
    fn builds_ordered_plan() {
        let dir = payload_dir("ordered");
        let (p, d) = plan(&dir);
        assert!(d.is_empty(), "{d:?}");
        let p = p.unwrap();
        let lines: Vec<String> = p.operations.iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "mkdir     {app}/bin",
                "mkdir     {app}/themes",
                "mkdir     {app}/themes/dark",
                "file      {app}/bin/aurea.exe (7 bytes)",
                "file      {app}/themes/dark/main.toml (4 bytes)",
                "file      {app}/themes/light.toml (5 bytes)",
                "path      {app}/bin (user)",
                "shortcut  Aurea -> {app}/bin/aurea.exe (applications)",
                "register  Aurea",
            ]
        );
        assert_eq!(p.payload_size(), 16);
        assert_eq!(p.components, ["cli", "core"]);
        // same input, same plan.
        assert_eq!(plan(&dir).0.unwrap(), p);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn optional_components_can_be_dropped_at_install() {
        let dir = payload_dir("choices");
        fs::create_dir_all(dir.join("docs/guide")).unwrap();
        fs::write(dir.join("docs/guide/intro.md"), b"intro").unwrap();
        let src = format!(
            "{AUREA}
component docs Documentation off
  copy docs
"
        );
        let m = Manifest::parse("Kagofile", &src).0.unwrap();
        let target: Target = "windows-x86_64".parse().unwrap();
        let all = Selection::with(&m, ["cli", "docs"]);
        let r = resolve(&m, &Context::target(target), all).0.unwrap();
        let p = InstallPlan::build(&m, &r, target, Scope::User, dir.as_path())
            .0
            .unwrap();
        let ids: Vec<(&str, bool)> = p
            .choices
            .iter()
            .map(|c| (c.id.as_str(), c.default))
            .collect();
        assert_eq!(
            ids,
            [("cli", true), ("docs", false)],
            "core is required, not a choice"
        );
        assert_eq!(p.choices[1].size, 5, "docs/guide/intro.md");

        let defaults = p.with_defaults();
        assert!(defaults.choices.is_empty());
        assert_eq!(defaults.components, ["cli", "core"]);
        let text = defaults.to_string();
        assert!(text.contains("path      {app}/bin"), "{text}");
        assert!(!text.contains("docs"), "docs folders go too: {text}");

        let bare = p.select([]);
        assert!(!bare.to_string().contains("path      "));
        assert_eq!(bare.components, ["core"]);

        let everything = p.select(["docs"]);
        assert!(
            everything
                .to_string()
                .contains("mkdir     {app}/docs/guide")
        );
        assert!(
            everything
                .to_string()
                .contains("file      {app}/docs/guide/intro.md")
        );
        assert!(!everything.to_string().contains("path      "));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hashes_payload() {
        let dir = payload_dir("hash");
        let (size, digest) = hash(&dir.join("themes/light.toml")).unwrap();
        assert_eq!(size, 5);
        assert_eq!(
            digest.to_string(),
            "99a7026172d42714d0293e5598ec1c8e8260d7d0c4b8582ad002c1883494e216"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_payload_is_an_error() {
        let dir = payload_dir("missing");
        fs::remove_file(dir.join("dist/windows-x86_64/aurea.exe")).unwrap();
        let (p, d) = plan(&dir);
        assert!(p.is_none());
        assert_eq!(d.iter().next().unwrap().code, diagnostic::INSTALL_SOURCE);
        fs::remove_dir_all(dir).unwrap();
    }
}
