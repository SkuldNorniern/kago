use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::journal::{Entry, Journal, Undo};
use super::recovery::{cleanup, roll_back};
use super::{Error, Store, remove_resources};
use crate::install::{InstallPlan, Need, Operation, Roots, Scope, at_least, hash};
use crate::manifest::{Downgrade, HookPhase, Ownership, Root, SemanticPath, ShortcutLocation};
use crate::platform::{Platform, Registration};
use crate::receipt::{self, Receipt, Resource};

/// runs `plan`, commits a receipt, removes what an older version owned and
/// the new one does not. on failure every step taken is undone.
pub(super) fn install(
    plan: &InstallPlan,
    platform: &dyn Platform,
    store: &Store,
    runtime: &Path,
    observe: &mut dyn FnMut(usize, usize, &Operation),
) -> Result<Receipt, Error> {
    let unmet = unmet(plan, platform);
    if !unmet.is_empty() {
        return Err(Error::Unmet(unmet));
    }
    let previous = store.read_receipt(&plan.package)?;
    if let Some(installed) = &previous
        && plan.downgrade == Downgrade::Deny
        && plan.version.cmp_precedence(&installed.version) == std::cmp::Ordering::Less
    {
        return Err(Error::Downgrade {
            installed: installed.version.to_string(),
            offered: plan.version.to_string(),
        });
    }
    let mut roots = platform
        .roots(plan.scope, &plan.name)
        .map_err(Error::Other)?;
    // a chosen folder, else wherever the installed version lives.
    let location = plan.location.clone().or_else(|| {
        previous
            .as_ref()
            .and_then(|p| p.roots.get(&Root::App).cloned())
    });
    if let Some(dir) = location {
        roots = roots.with_app(&dir);
    }
    let mut journal = Journal::begin(
        &store.journal_path(&plan.package),
        &plan.package,
        &plan.version,
    )?;
    let mut run = Run {
        plan,
        platform,
        store,
        roots: &roots,
        runtime,
        observe,
        journal: &mut journal,
        undos: Vec::new(),
        resources: Vec::new(),
        backups: 0,
        reregister: previous.as_ref().is_some_and(|p| {
            p.resources
                .iter()
                .any(|r| matches!(r, Resource::Registration { .. }))
        }),
        previous: previous.as_ref(),
        transaction: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
            .to_string(),
    };
    if let Err(e) = run.all() {
        let failures = roll_back(&run.undos, platform);
        drop(journal);
        if failures.is_empty() {
            cleanup(store, &plan.package)?;
            return Err(Error::RolledBack(e.to_string()));
        }
        return Err(Error::Other(format!(
            "{e}\nrollback was incomplete, run `kago recover`:\n  {}",
            failures.join("\n  ")
        )));
    }
    let resources = std::mem::take(&mut run.resources);
    let used: Vec<SemanticPath> = resources
        .iter()
        .filter_map(|r| match r {
            Resource::Directory { path } | Resource::File { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    let receipt = Receipt {
        schema: receipt::SCHEMA,
        package: plan.package.clone(),
        name: plan.name.clone(),
        version: plan.version.clone(),
        publisher: plan.publisher.clone(),
        target: plan.target,
        scope: plan.scope,
        components: plan.components.clone(),
        roots: roots.used(&used),
        resources,
    };
    store.write_receipt(&receipt)?;
    journal.write(&Entry::Commit)?;
    drop(journal);
    // committed. a backup still held open (the old exe of a running app)
    // keeps the journal around; the next kago run finishes the cleanup.
    let _ = cleanup(store, &plan.package);

    if let Some(previous) = previous {
        drop_stale(&previous, &receipt, platform);
    }
    Ok(receipt)
}

/// removes what the old version owned and the new one does not. best
/// effort: the new version is committed either way. its uninstall hooks do
/// not run: an upgrade replaces the package, it does not uninstall it.
fn drop_stale(previous: &Receipt, receipt: &Receipt, platform: &dyn Platform) {
    let stale: Vec<Resource> = previous
        .resources
        .iter()
        .filter(|r| !same_place(r, previous, receipt))
        .filter(|r| !matches!(r, Resource::UninstallHook { .. }))
        .cloned()
        .collect();
    let _ = remove_resources(previous, &stale, platform);
}

/// the needs of `plan` this machine does not meet, as sentences.
fn unmet(plan: &InstallPlan, platform: &dyn Platform) -> Vec<String> {
    plan.needs
        .iter()
        .filter_map(|need| {
            let met = match need {
                Need::Os { os, minimum } => {
                    *os != plan.target.os()
                        || platform
                            .os_version()
                            .is_some_and(|have| at_least(&have, minimum))
                }
                Need::Software { detect } => platform.has(*detect),
            };
            (!met).then(|| need.to_string())
        })
        .collect()
}

/// whether the new receipt still owns what `old` names, maybe under
/// another hash or size, like a replaced file. then it must not be deleted.
/// files and folders compare on disk: the same `{app}/x` in a folder the
/// install moved away from is stale.
fn same_place(old: &Resource, previous: &Receipt, new: &Receipt) -> bool {
    let on_disk =
        |a: &SemanticPath, b: &SemanticPath| a == b && previous.absolute(a) == new.absolute(b);
    new.resources.iter().any(|n| match (old, n) {
        (Resource::File { path: a, .. }, Resource::File { path: b, .. })
        | (Resource::Directory { path: a }, Resource::Directory { path: b }) => on_disk(a, b),
        (Resource::Registration { key: a, .. }, Resource::Registration { key: b, .. })
        | (Resource::PathEntry { entry: a, .. }, Resource::PathEntry { entry: b, .. })
        | (Resource::LoginItem { name: a }, Resource::LoginItem { name: b })
        | (Resource::SteamVrDriver { directory: a }, Resource::SteamVrDriver { directory: b }) => {
            a == b
        }
        (Resource::Shortcut { path: a }, Resource::Shortcut { path: b }) => a == b,
        (
            Resource::UninstallHook {
                command: a,
                arguments: x,
            },
            Resource::UninstallHook {
                command: b,
                arguments: y,
            },
        ) => a == b && x == y,
        _ => false,
    })
}

struct Run<'a> {
    plan: &'a InstallPlan,
    platform: &'a dyn Platform,
    store: &'a Store,
    roots: &'a Roots,
    runtime: &'a Path,
    /// told before each operation: its index, the count and the operation.
    observe: &'a mut dyn FnMut(usize, usize, &Operation),
    journal: &'a mut Journal,
    undos: Vec<Undo>,
    resources: Vec<Resource>,
    backups: usize,
    /// an older version is registered; overwrite without owning a delete.
    reregister: bool,
    /// the older version's receipt, to tell user changes from its files.
    previous: Option<&'a Receipt>,
    /// unique per run, so backups never land on files an earlier run
    /// could not delete yet.
    transaction: String,
}

impl Run<'_> {
    fn all(&mut self) -> Result<(), Error> {
        let mut used_roots = BTreeSet::new();
        for op in &self.plan.operations {
            match op {
                Operation::CreateDirectory { path } => {
                    used_roots.insert(path.root);
                }
                Operation::InstallFile { target, .. } => {
                    used_roots.insert(target.root);
                }
                _ => {}
            }
        }
        for root in used_roots {
            self.directory(&SemanticPath::from(root))?;
        }
        let total = self.plan.operations.len();
        for (i, op) in self.plan.operations.iter().enumerate() {
            (self.observe)(i, total, op);
            self.op(op)?;
        }
        Ok(())
    }

    fn step(&mut self, undo: Undo) -> Result<(), Error> {
        self.journal.step(undo.clone())?;
        self.undos.push(undo);
        Ok(())
    }

    fn resolve(&self, path: &SemanticPath) -> Result<PathBuf, Error> {
        self.roots.resolve(path).map_err(Error::Other)
    }

    fn op(&mut self, op: &Operation) -> Result<(), Error> {
        match op {
            Operation::CreateDirectory { path } => self.directory(path),
            Operation::InstallFile {
                source,
                target,
                size,
                sha256,
                ownership,
            } => {
                let dest = self.resolve(target)?;
                if let Some(kept) = self.keep_user_file(target, &dest, *ownership) {
                    self.resources.push(Resource::File {
                        path: target.clone(),
                        size: *size,
                        sha256: kept,
                        ownership: *ownership,
                    });
                    return Ok(());
                }
                self.place(&dest, |dest| {
                    fs::copy(source, dest).map_err(|e| Error::io(dest, &e))?;
                    let (got_size, got) = hash(dest).map_err(|e| Error::io(dest, &e))?;
                    if got != *sha256 || got_size != *size {
                        return Err(Error::Other(format!(
                            "{} changed since the plan was built",
                            source.display()
                        )));
                    }
                    Ok(())
                })?;
                self.resources.push(Resource::File {
                    path: target.clone(),
                    size: *size,
                    sha256: *sha256,
                    ownership: *ownership,
                });
                Ok(())
            }
            Operation::AddPathEntry { directory, scope } => self.path_entry(directory, *scope),
            Operation::RegisterSteamVrDriver { directory } => self.steamvr_driver(directory),
            Operation::RunHook {
                phase,
                command,
                arguments,
            } => self.hook(*phase, command, arguments),
            Operation::CreateShortcut {
                name,
                target,
                location,
                arguments,
                icon,
            } => {
                let icon = icon.as_ref().map(|i| self.resolve(i)).transpose()?;
                let root = match location {
                    ShortcutLocation::Applications => Root::Applications,
                    ShortcutLocation::Desktop => Root::Desktop,
                };
                let dir = self.resolve(&SemanticPath::from(root))?;
                fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, &e))?;
                let link = dir.join(self.platform.shortcut_file(name));
                let target = self.resolve(target)?;
                let platform = self.platform;
                self.place(&link, |link| {
                    platform
                        .create_shortcut(link, &target, arguments, icon.as_deref())
                        .map_err(Error::Other)
                })?;
                self.resources.push(Resource::Shortcut { path: link });
                Ok(())
            }
            Operation::RegisterApplication {
                display_name,
                entrypoint,
                icon,
            } => self.register(display_name, entrypoint.as_ref(), icon.as_ref()),
            Operation::InstallService {
                id,
                command,
                arguments,
                scope,
            } => self.login_item(id, command, arguments, *scope),
        }
    }

    /// an existing file the user may own: user data always stays, a
    /// preserve-if-modified file stays when it differs from what the older
    /// version installed (or nothing recorded it). returns the hash to record,
    /// the older one, so uninstall still sees the change.
    fn path_entry(&mut self, directory: &SemanticPath, scope: Scope) -> Result<(), Error> {
        let entry = self.resolve(directory)?;
        let text = entry.display().to_string();
        let owned_before = self.previous.is_some_and(|p| {
            p.resources
                .iter()
                .any(|r| matches!(r, Resource::PathEntry { entry, .. } if *entry == text))
        });
        // tiny window: a crash between add and the journal write
        // leaves the entry; it is never someone else's entry.
        let added = self
            .platform
            .add_path_entry(scope, &entry)
            .map_err(Error::Other)?;
        if added {
            self.step(Undo::RemovePathEntry {
                entry: entry.clone(),
                scope,
            })?;
        }
        // a reinstall finds its own entry there; it stays ours, or
        // the cleanup of the old receipt would take it off PATH.
        if added || owned_before {
            self.resources
                .push(Resource::PathEntry { entry: text, scope });
        }
        Ok(())
    }

    /// a per-user service: started when the user signs in, under a name
    /// made from the package id, so it is never someone else's.
    fn login_item(
        &mut self,
        id: &str,
        command: &SemanticPath,
        arguments: &[String],
        scope: Scope,
    ) -> Result<(), Error> {
        if scope == Scope::Machine {
            return Err(Error::Other(format!(
                "service `{id}` for every user needs the elevation helper, which is not done yet"
            )));
        }
        let exe = self.resolve(command)?;
        let name = format!("{}.{id}", self.plan.package);
        let previous = self.platform.login_item(&name).map_err(Error::Other)?;
        self.step(Undo::RestoreLoginItem {
            name: name.clone(),
            previous,
        })?;
        self.platform
            .add_login_item(&name, &exe, arguments)
            .map_err(Error::Other)?;
        self.resources.push(Resource::LoginItem { name });
        Ok(())
    }

    /// a post-install hook runs now; what it did cannot be taken back, but
    /// a failing one still rolls the install itself back. a pre-uninstall
    /// hook is kept for uninstall.
    fn hook(
        &mut self,
        phase: HookPhase,
        command: &SemanticPath,
        arguments: &[String],
    ) -> Result<(), Error> {
        let exe = self.resolve(command)?;
        if phase == HookPhase::PreUninstall {
            self.resources.push(Resource::UninstallHook {
                command: exe.display().to_string(),
                arguments: arguments.to_vec(),
            });
            return Ok(());
        }
        super::hook::run(&exe, arguments).map_err(Error::Other)
    }

    fn steamvr_driver(&mut self, directory: &SemanticPath) -> Result<(), Error> {
        let dir = self.resolve(directory)?;
        let entry = dir.display().to_string();
        let owned_before = self.previous.is_some_and(|p| {
            p.resources
                .iter()
                .any(|r| matches!(r, Resource::SteamVrDriver { directory } if *directory == entry))
        });
        if self
            .platform
            .add_steamvr_driver(&dir)
            .map_err(Error::Other)?
        {
            self.step(Undo::RemoveSteamVrDriver {
                directory: dir.clone(),
            })?;
            self.resources
                .push(Resource::SteamVrDriver { directory: entry });
        } else if owned_before {
            // an upgrade: still ours, though this run did not add it.
            self.resources
                .push(Resource::SteamVrDriver { directory: entry });
        }
        Ok(())
    }

    fn keep_user_file(
        &self,
        target: &SemanticPath,
        dest: &Path,
        ownership: Ownership,
    ) -> Option<crate::install::Digest> {
        if !dest.is_file() {
            return None;
        }
        let recorded = self.previous.and_then(|p| {
            p.resources.iter().find_map(|r| match r {
                Resource::File { path, sha256, .. } if path == target => Some(*sha256),
                _ => None,
            })
        });
        let (_, now) = hash(dest).ok()?;
        match ownership {
            Ownership::UserData => Some(recorded.unwrap_or(now)),
            Ownership::PreserveIfModified if recorded != Some(now) => {
                // recorded hash, or a never-matching one for a file kago did
                // not put there, so uninstall keeps it.
                Some(recorded.unwrap_or(crate::install::Digest([0; 32])))
            }
            _ => None,
        }
    }

    /// creates one directory and owns it, or leaves an existing one alone.
    fn directory(&mut self, path: &SemanticPath) -> Result<(), Error> {
        let dir = self.resolve(path)?;
        if dir.is_dir() {
            // an upgrade keeps owning what the older version created.
            let owned = self.previous.is_some_and(|p| {
                p.resources
                    .iter()
                    .any(|r| matches!(r, Resource::Directory { path: owned } if owned == path))
            });
            if owned {
                self.resources
                    .push(Resource::Directory { path: path.clone() });
            }
            return Ok(());
        }
        if let Some(parent) = dir.parent()
            && !parent.is_dir()
        {
            fs::create_dir_all(parent).map_err(|e| Error::io(parent, &e))?;
        }
        self.step(Undo::RemoveDirectory { path: dir.clone() })?;
        fs::create_dir(&dir).map_err(|e| Error::io(&dir, &e))?;
        self.resources
            .push(Resource::Directory { path: path.clone() });
        Ok(())
    }

    /// writes `dest` with `write`, moving an existing file aside first so
    /// rollback can put it back.
    fn place(
        &mut self,
        dest: &Path,
        write: impl FnOnce(&Path) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if dest.exists() {
            let backup = self
                .store
                .backup_dir(&self.plan.package)
                .join(&self.transaction)
                .join(self.backups.to_string());
            self.backups += 1;
            let dir = backup.parent().expect("backup has a parent");
            fs::create_dir_all(dir).map_err(|e| Error::io(dir, &e))?;
            self.step(Undo::RestoreFile {
                path: dest.to_path_buf(),
                backup: backup.clone(),
            })?;
            move_file(dest, &backup)?;
        } else {
            self.step(Undo::RemoveFile {
                path: dest.to_path_buf(),
            })?;
        }
        write(dest)
    }

    fn register(
        &mut self,
        display_name: &str,
        entrypoint: Option<&SemanticPath>,
        icon: Option<&SemanticPath>,
    ) -> Result<(), Error> {
        let runtime = self.store.runtime_path();
        copy_runtime(self.runtime, &runtime)?;
        let scope: Scope = self.plan.scope;
        let registration = Registration {
            package: self.plan.package.clone(),
            display_name: display_name.to_owned(),
            version: self.plan.version.clone(),
            publisher: self.plan.publisher.clone(),
            install_location: self.resolve(&SemanticPath::from(Root::App))?,
            icon: icon.or(entrypoint).map(|e| self.resolve(e)).transpose()?,
            uninstall: format!(
                "\"{}\" uninstall {} --scope {scope}",
                runtime.display(),
                self.plan.package
            ),
            size: self.plan.payload_size(),
        };
        let key = self
            .platform
            .register(scope, &registration)
            .map_err(Error::Other)?;
        if !self.reregister {
            self.step(Undo::RemoveRegistration {
                key: key.clone(),
                scope,
            })?;
        }
        self.resources.push(Resource::Registration { key, scope });
        Ok(())
    }
}

/// rename, or copy and delete across volumes.
fn move_file(from: &Path, to: &Path) -> Result<(), Error> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to).map_err(|e| Error::io(to, &e))?;
    fs::remove_file(from).map_err(|e| Error::io(from, &e))
}

/// keeps one copy of the running kago where uninstall entries point. the
/// running file itself is never replaced.
fn copy_runtime(current: &Path, runtime: &Path) -> Result<(), Error> {
    if current == runtime || same_file(current, runtime) {
        return Ok(());
    }
    let dir = runtime.parent().expect("runtime has a parent");
    fs::create_dir_all(dir).map_err(|e| Error::io(dir, &e))?;
    let tmp = runtime.with_extension("new");
    fs::copy(current, &tmp).map_err(|e| Error::io(&tmp, &e))?;
    match fs::rename(&tmp, runtime) {
        Ok(()) => Ok(()),
        // the runtime is running (uninstalling something); keep it.
        Err(_) if runtime.exists() => {
            let _ = fs::remove_file(&tmp);
            Ok(())
        }
        Err(e) => Err(Error::io(runtime, &e)),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((hash(a), hash(b)), (Ok(x), Ok(y)) if x == y)
}
