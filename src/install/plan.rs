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
use crate::manifest::{
    Check, Detect, Downgrade, File, HookPhase, HookPrivilege, Manifest, ResourceScope, ScopeChoice,
    SemanticPath, SetupLook,
};
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
    /// the person installing picks between everyone and just themselves;
    /// `scope` is where the pick starts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scope_open: bool,
    pub components: Vec<String>,
    /// whether this may replace a newer installed version.
    pub downgrade: Downgrade,
    pub operations: Vec<Operation>,
    /// optional components still open to the person installing, with the
    /// operations each brings. empty once [`InstallPlan::select`] ran.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    /// ids of the optional components there were to pick, still known
    /// after [`InstallPlan::select`], so a receipt can tell a component
    /// left out from one that is new.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub offered: Vec<String>,
    /// how the setup looked, so the uninstaller looks the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<Look>,
    /// other vendors' installers to run first, when missing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prerequisites: Vec<Prerequisite>,
    /// URL of the update feed, kept in the receipt for `kago update`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<String>,
    /// `{app}` somewhere other than the platform's default, chosen at
    /// install. upgrades keep the folder of the install they replace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<PathBuf>,
    /// what the machine must have before anything changes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<Need>,
}

/// the look of the setup and of the uninstaller it leaves.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Look {
    pub setup: SetupLook,
    /// the brand's main color, as RGB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<[u8; 3]>,
    /// the brand's small text at the bottom left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// another vendor's installer the install runs first when its check
/// fails. inside a setup the installer is the asset
/// [`Prerequisite::asset`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Prerequisite {
    pub id: String,
    pub name: String,
    /// build-side installer; empty inside a setup.
    #[serde(default, skip_serializing_if = "is_empty_path")]
    pub source: PathBuf,
    /// what to call it when it runs: some installers read their own name.
    pub file_name: String,
    pub arguments: Vec<String>,
    pub check: Check,
    /// its installer needs administrator rights.
    pub admin: bool,
}

impl Prerequisite {
    /// the setup asset holding its installer.
    #[must_use]
    pub fn asset(&self) -> String {
        format!("prerequisite:{}", self.id)
    }
}

fn is_empty_path(path: &Path) -> bool {
    path.as_os_str().is_empty()
}

/// an optional component the setup can offer as a tick box.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Choice {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// ticked unless the person unticks it.
    pub default: bool,
    /// always installed; shown ticked and locked.
    #[serde(default)]
    pub required: bool,
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

        let mut steps = Steps::default();
        for path in directories {
            steps.push(Operation::CreateDirectory { path }, None);
        }
        for f in files {
            let op = Operation::InstallFile {
                source: f.source,
                target: f.target,
                size: f.size,
                sha256: f.sha256,
                ownership: f.ownership,
            };
            steps.push(op, f.component);
        }
        // the main app's installed .ico, when it has one.
        let icon = manifest.application.entrypoint.as_ref().and_then(|id| {
            let icon_id = format!("{id}-icon");
            resolved
                .files
                .iter()
                .find(|f| f.logical_id.as_deref() == Some(icon_id.as_str()))
                .map(|f| f.target.clone())
        });
        let scope_open = manifest.install.scope == ScopeChoice::Choose;
        if scope_open {
            fixed_with_open_scope(resolved, &mut diagnostics);
        }
        steps.resources(resolved, scope, icon.as_ref(), manifest, &mut diagnostics);
        let entrypoint = manifest.application.entrypoint.as_ref().and_then(|id| {
            resolved
                .files
                .iter()
                .find(|f| f.logical_id.as_ref() == Some(id))
                .map(|f| f.target.clone())
        });
        let register = Operation::RegisterApplication {
            display_name: manifest.application.display_name.clone(),
            entrypoint,
            icon,
        };
        steps.push(register, None);
        for op in hooks(resolved, &mut diagnostics) {
            steps.push(op, None);
        }
        let needs = needs(resolved, &mut diagnostics);
        let prerequisites = prerequisites(manifest, target, sources, &mut diagnostics);
        if diagnostics.has_errors() {
            return (None, diagnostics);
        }
        let choices = choices(manifest, resolved, &steps.owners, &steps.operations);
        let operations = steps.operations;
        let plan = Self {
            package: manifest.package.id.clone(),
            name: manifest.package.name.clone(),
            version: manifest.package.version.clone(),
            publisher: manifest.package.publisher.clone(),
            target,
            scope,
            scope_open,
            components: resolved.components.iter().map(str::to_owned).collect(),
            downgrade: manifest.install.downgrade,
            operations,
            choices,
            offered: Vec::new(),
            look: Some(Look {
                setup: manifest.install.look,
                accent: manifest.brand.accent,
                text: manifest.brand.text.clone(),
            }),
            prerequisites,
            update: manifest.update.as_ref().map(|u| u.feed.clone()),
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
        let required = self
            .choices
            .iter()
            .filter(|c| c.required)
            .map(|c| c.id.clone());
        let mut todo: Vec<String> = chosen
            .into_iter()
            .map(str::to_owned)
            .chain(required)
            .collect();
        while let Some(id) = todo.pop() {
            if let Some(choice) = self.choices.iter().find(|c| c.id == id)
                && on.insert(choice.id.as_str())
            {
                todo.extend(choice.depends.iter().cloned());
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
        let offered = if self.choices.is_empty() {
            self.offered.clone()
        } else {
            self.choices.iter().map(|c| c.id.clone()).collect()
        };
        Self {
            components,
            operations,
            choices: Vec::new(),
            offered,
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

    /// the plan installing for everyone or just this user, when the person
    /// installing may pick. PATH entries follow; `None` when the scope is
    /// fixed and `scope` is not it.
    #[must_use]
    pub fn in_scope(&self, scope: Scope) -> Option<Self> {
        if scope == self.scope {
            return Some(self.clone());
        }
        if !self.scope_open {
            return None;
        }
        let operations = self
            .operations
            .iter()
            .map(|op| match op {
                Operation::AddPathEntry { directory, .. } => Operation::AddPathEntry {
                    directory: directory.clone(),
                    scope,
                },
                op => op.clone(),
            })
            .collect();
        Some(Self {
            scope,
            operations,
            ..self.clone()
        })
    }

    /// the optional components an update or repair keeps: those installed,
    /// not those left out, and the defaults for ones that are new.
    /// `installed` and `offered` come from the receipt.
    #[must_use]
    pub fn kept_choices(&self, installed: &[String], offered: &[String]) -> Vec<String> {
        self.choices
            .iter()
            .filter(|c| installed.contains(&c.id) || (c.default && !offered.contains(&c.id)))
            .map(|c| c.id.clone())
            .collect()
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
        if self.scope_open {
            writeln!(
                f,
                "scope: {}, or the other when picked at install",
                self.scope
            )?;
        } else {
            writeln!(f, "scope: {}", self.scope)?;
        }
        writeln!(f, "components: {}", self.components.join(", "))?;
        writeln!(f, "payload: {} bytes", self.payload_size())?;
        writeln!(f)?;
        for op in &self.operations {
            writeln!(f, "  {op}")?;
        }
        Ok(())
    }
}

/// operations in order, each with the component that brings it.
#[derive(Default)]
struct Steps {
    operations: Vec<Operation>,
    /// in step with `operations`.
    owners: Vec<Option<String>>,
}

impl Steps {
    fn push(&mut self, op: Operation, owner: Option<String>) {
        self.operations.push(op);
        self.owners.push(owner);
    }

    /// PATH entries, SteamVR drivers, shortcuts and services.
    fn resources(
        &mut self,
        resolved: &Resolved,
        scope: Scope,
        icon: Option<&SemanticPath>,
        manifest: &Manifest,
        diagnostics: &mut Diagnostics,
    ) {
        for p in &resolved.paths {
            let op = Operation::AddPathEntry {
                directory: p.directory.clone(),
                scope: scope.of(p.scope),
            };
            self.push(op, p.component.clone());
        }
        for driver in &resolved.steamvr_drivers {
            if let Err(d) = driver_shipped(driver, &self.operations) {
                diagnostics.push(d);
            }
            let op = Operation::RegisterSteamVrDriver {
                directory: driver.directory.clone(),
            };
            self.push(op, driver.component.clone());
        }
        for s in &resolved.shortcuts {
            // the main app's icon goes on shortcuts to the main app.
            let main =
                manifest.application.entrypoint.as_deref() == Some(s.shortcut.target.as_str());
            let op = Operation::CreateShortcut {
                name: s.shortcut.name.clone(),
                target: s.target.clone(),
                location: s.shortcut.location,
                arguments: s.shortcut.arguments.clone(),
                icon: icon.filter(|_| main).cloned(),
            };
            self.push(op, s.shortcut.component.clone());
        }
        for s in &resolved.services {
            if scope.of(s.service.scope) == Scope::Machine {
                diagnostics.push(
                    Diagnostic::error(
                        diagnostic::TARGET_CAPABILITY,
                        format!(
                            "service `{}` for every user needs the elevation helper, which is not done yet",
                            s.service.id
                        ),
                    )
                    .at(s.service.at.clone())
                    .help("add `scope user` under it: it starts when the person installing signs in"),
                );
                continue;
            }
            let op = Operation::InstallService {
                id: s.service.id.clone(),
                command: s.command.clone(),
                arguments: s.service.arguments.clone(),
                scope: scope.of(s.service.scope),
            };
            self.push(op, s.service.component.clone());
        }
    }
}

/// with `scope choose`, PATH entries follow the pick, and a service has
/// to say it is per-user: one for everyone is not done yet.
fn fixed_with_open_scope(resolved: &Resolved, diagnostics: &mut Diagnostics) {
    for p in &resolved.paths {
        if p.scope != ResourceScope::ResolvedInstall {
            diagnostics.push(Diagnostic::error(
                diagnostic::INSTALL_SCOPE,
                format!(
                    "PATH entry `{}` has its own scope, but with `scope choose` it follows the person's pick",
                    p.directory
                ),
            ));
        }
    }
    for s in &resolved.services {
        if s.service.scope == ResourceScope::ResolvedInstall {
            diagnostics.push(
                Diagnostic::error(
                    diagnostic::INSTALL_SCOPE,
                    format!(
                        "service `{}` would run for everyone when the person picks that, which is not done yet",
                        s.service.id
                    ),
                )
                .at(s.service.at.clone())
                .help("add `scope user` under it: it starts when the person installing signs in"),
            );
        }
    }
}

/// the manifest's prerequisites with their installers found. Windows only
/// for now: elsewhere they are left out with a warning.
fn prerequisites<S: Sources + ?Sized>(
    manifest: &Manifest,
    target: Target,
    sources: &S,
    diagnostics: &mut Diagnostics,
) -> Vec<Prerequisite> {
    if manifest.prerequisites.is_empty() {
        return Vec::new();
    }
    if target.os() != crate::package::Os::Windows {
        diagnostics.push(Diagnostic::warning(
            diagnostic::TARGET_CAPABILITY,
            format!("prerequisites run on Windows only; left out for {target}"),
        ));
        return Vec::new();
    }
    let mut out = Vec::new();
    for p in &manifest.prerequisites {
        match sources.locate(&p.file) {
            Ok(source) if source.is_file() => out.push(Prerequisite {
                id: p.id.clone(),
                name: p.name.clone(),
                file_name: source
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("setup.exe")
                    .to_owned(),
                source,
                arguments: p.arguments.clone(),
                check: p.check.clone(),
                admin: p.admin,
            }),
            Ok(source) => diagnostics.push(
                Diagnostic::error(
                    diagnostic::MANIFEST_REFERENCE,
                    format!(
                        "prerequisite `{}`: {} does not exist",
                        p.id,
                        source.display()
                    ),
                )
                .at(p.at.clone()),
            ),
            Err(e) => diagnostics.push(
                Diagnostic::error(
                    diagnostic::MANIFEST_REFERENCE,
                    format!("prerequisite `{}`: {e}", p.id),
                )
                .at(p.at.clone()),
            ),
        }
    }
    out
}

/// the resolved hooks kago can run, with a diagnostic for each it cannot.
fn hooks(resolved: &Resolved, diagnostics: &mut Diagnostics) -> Vec<Operation> {
    let mut out = Vec::new();
    for h in &resolved.hooks {
        match hook(h) {
            Ok(op) => out.push(op),
            Err(d) => diagnostics.push(d),
        }
    }
    out
}

/// a hook as an operation, when kago can run it at that phase yet.
fn hook(h: &super::ResolvedHook) -> Result<Operation, Diagnostic> {
    let unsupported =
        |what: String| Diagnostic::error(diagnostic::TARGET_CAPABILITY, what).at(h.hook.at.clone());
    if h.hook.privilege == HookPrivilege::Elevated {
        return Err(unsupported(
            "elevated hooks need the elevation helper, which is not done yet".to_owned(),
        ));
    }
    if !matches!(
        h.hook.phase,
        HookPhase::PostInstall | HookPhase::PreUninstall
    ) {
        return Err(unsupported(format!(
            "`{}` hooks are not supported yet; use post-install or pre-uninstall",
            h.hook.phase
        )));
    }
    Ok(Operation::RunHook {
        phase: h.hook.phase,
        command: h.command.clone(),
        arguments: h.hook.arguments.clone(),
    })
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
        .filter(|c| resolved.components.contains(Some(&c.id)))
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
                required: c.required,
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
    fn an_open_scope_moves_path_entries_with_the_pick() {
        let dir = payload_dir("scope");
        let p = plan(&dir).0.unwrap();
        assert!(p.scope_open, "the example says `scope choose`");
        let everyone = p.in_scope(Scope::Machine).unwrap();
        assert_eq!(everyone.scope, Scope::Machine);
        assert!(everyone.operations.iter().any(|op| matches!(
            op,
            Operation::AddPathEntry {
                scope: Scope::Machine,
                ..
            }
        )));
        let fixed = InstallPlan {
            scope_open: false,
            ..p
        };
        assert!(fixed.in_scope(Scope::Machine).is_none());
        assert!(fixed.in_scope(Scope::User).is_some());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn updates_keep_what_was_picked_and_default_new_parts() {
        let choice = |id: &str, default| Choice {
            id: id.into(),
            name: id.into(),
            description: None,
            default,
            required: false,
            depends: Vec::new(),
            operations: Vec::new(),
            size: 0,
            detect: None,
        };
        let dir = payload_dir("kept");
        let mut p = plan(&dir).0.unwrap();
        p.choices = vec![
            choice("desktop", false),
            choice("docs", true),
            choice("new-on", true),
            choice("new-off", false),
        ];
        let installed = ["desktop".to_owned()];
        let offered = ["desktop".to_owned(), "docs".to_owned()];
        assert_eq!(
            p.kept_choices(&installed, &offered),
            ["desktop", "new-on"],
            "docs was left out"
        );
        assert_eq!(p.select(["desktop"]).offered.len(), 4);
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
        assert_eq!(ids, [("core", true), ("cli", true), ("docs", false)]);
        assert!(p.choices[0].required, "core is shown, locked");
        assert_eq!(p.choices[2].size, 5, "docs/guide/intro.md");

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
    fn only_post_install_and_pre_uninstall_hooks_run_for_now() {
        let dir = payload_dir("hooks");
        let build = |extra: &str| {
            let src = format!(
                "{AUREA}
{extra}"
            );
            let m = Manifest::parse("Kagofile", &src).0.unwrap();
            let target: Target = "windows-x86_64".parse().unwrap();
            let r = resolve(&m, &Context::target(target), Selection::defaults(&m))
                .0
                .unwrap();
            InstallPlan::build(&m, &r, target, Scope::User, dir.as_path())
        };
        let (p, d) = build(
            "hook post-install main-executable --setup
",
        );
        assert!(d.is_empty(), "{d:?}");
        assert!(
            p.unwrap()
                .to_string()
                .contains("hook      post-install {app}/bin/aurea.exe")
        );
        let (p, d) = build(
            "hook pre-install main-executable
",
        );
        assert!(p.is_none());
        assert_eq!(
            d.iter().map(|d| d.code).collect::<Vec<_>>(),
            [diagnostic::TARGET_CAPABILITY]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_app_icon_is_installed_for_shortcuts_and_the_uninstall_entry() {
        let dir = std::env::temp_dir().join(format!("kago-plan-icon-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("tool.exe"), b"MZ").unwrap();
        fs::write(dir.join("tool.ico"), b"ico").unwrap();
        let src = "package dev.x.tool
  version 1.0.0
app tool
  windows tool.exe
  icon windows tool.ico
shortcut
";
        let m = Manifest::parse("Kagofile", src).0.unwrap();
        let target: Target = "windows-x86_64".parse().unwrap();
        let r = resolve(&m, &Context::target(target), Selection::defaults(&m))
            .0
            .unwrap();
        let p = InstallPlan::build(&m, &r, target, Scope::User, dir.as_path())
            .0
            .unwrap();
        let icon: SemanticPath = "{app}/tool.ico".parse().unwrap();
        assert!(
            p.operations
                .iter()
                .any(|op| matches!(op, Operation::InstallFile { target, .. } if *target == icon))
        );
        assert!(
            p.operations.iter().any(
                |op| matches!(op, Operation::CreateShortcut { icon: Some(i), .. } if *i == icon)
            )
        );
        assert!(p.operations.iter().any(
            |op| matches!(op, Operation::RegisterApplication { icon: Some(i), .. } if *i == icon)
        ));
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
