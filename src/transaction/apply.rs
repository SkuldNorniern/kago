use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::journal::{Entry, Journal, Undo};
use super::recovery::{cleanup, roll_back};
use super::{Error, Store, remove_resources};
use crate::install::{InstallPlan, Operation, Roots, Scope, hash};
use crate::manifest::{Downgrade, Ownership, Root, SemanticPath, ShortcutLocation};
use crate::platform::{Platform, Registration};
use crate::receipt::{self, Receipt, Resource};

/// runs `plan`, commits a receipt, removes what an older version owned and
/// the new one does not. on failure every step taken is undone.
pub(super) fn install(
    plan: &InstallPlan,
    platform: &dyn Platform,
    store: &Store,
    runtime: &Path,
) -> Result<Receipt, Error> {
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
    let roots = platform
        .roots(plan.scope, &plan.name)
        .map_err(Error::Other)?;
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
    cleanup(store, &plan.package)?;

    if let Some(previous) = previous {
        let stale: Vec<Resource> = previous
            .resources
            .iter()
            .filter(|r| !receipt.resources.contains(r) && !same_place(r, &receipt))
            .cloned()
            .collect();
        // best effort: the new version is committed either way.
        let _ = remove_resources(&previous, &stale, platform);
    }
    Ok(receipt)
}

/// a stale resource that the new receipt still owns under another hash or
/// size, like a replaced file. it must not be deleted.
fn same_place(old: &Resource, new: &Receipt) -> bool {
    new.resources.iter().any(|n| match (old, n) {
        (Resource::File { path: a, .. }, Resource::File { path: b, .. })
        | (Resource::Directory { path: a }, Resource::Directory { path: b }) => a == b,
        (Resource::Registration { key: a, .. }, Resource::Registration { key: b, .. })
        | (Resource::PathEntry { entry: a, .. }, Resource::PathEntry { entry: b, .. }) => a == b,
        (Resource::Shortcut { path: a }, Resource::Shortcut { path: b }) => a == b,
        _ => false,
    })
}

struct Run<'a> {
    plan: &'a InstallPlan,
    platform: &'a dyn Platform,
    store: &'a Store,
    roots: &'a Roots,
    runtime: &'a Path,
    journal: &'a mut Journal,
    undos: Vec<Undo>,
    resources: Vec<Resource>,
    backups: usize,
    /// an older version is registered; overwrite without owning a delete.
    reregister: bool,
    /// the older version's receipt, to tell user changes from its files.
    previous: Option<&'a Receipt>,
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
        for op in &self.plan.operations {
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
            Operation::AddPathEntry { directory, scope } => {
                let entry = self.resolve(directory)?;
                // tiny window: a crash between add and the journal write
                // leaves the entry; it is never someone else's entry.
                if self
                    .platform
                    .add_path_entry(*scope, &entry)
                    .map_err(Error::Other)?
                {
                    self.step(Undo::RemovePathEntry {
                        entry: entry.clone(),
                        scope: *scope,
                    })?;
                    self.resources.push(Resource::PathEntry {
                        entry: entry.display().to_string(),
                        scope: *scope,
                    });
                }
                Ok(())
            }
            Operation::CreateShortcut {
                name,
                target,
                location,
                arguments,
            } => {
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
                        .create_shortcut(link, &target, arguments)
                        .map_err(Error::Other)
                })?;
                self.resources.push(Resource::Shortcut { path: link });
                Ok(())
            }
            Operation::RegisterApplication {
                display_name,
                entrypoint,
            } => self.register(display_name, entrypoint.as_ref()),
            Operation::InstallService { id, .. } => Err(Error::Other(format!(
                "service `{id}`: installing services is not supported yet"
            ))),
        }
    }

    /// an existing file the user may own: user data always stays, a
    /// preserve-if-modified file stays when it differs from what the older
    /// version installed (or nothing recorded it). returns the hash to record,
    /// the older one, so uninstall still sees the change.
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
            icon: entrypoint.map(|e| self.resolve(e)).transpose()?,
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
