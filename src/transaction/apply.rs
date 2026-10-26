use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::journal::{Entry, Journal, Undo};
use super::recovery::{cleanup, move_file, roll_back};
use super::{Error, Store, remove_resources};
use crate::install::{InstallPlan, Need, Operation, Roots, Scope, at_least, hash};
use crate::manifest::{
    Downgrade, Edit, Ownership, Place, Root, RunAt, SemanticPath, ShortcutLocation, Value,
};
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
        let undos = std::mem::take(&mut run.undos);
        drop(journal);
        return Err(undone(&e, &undos, platform, store, &plan.package));
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
        offered: plan.offered.clone(),
        look: plan.look.clone(),
        update: plan.update.clone(),
        answers: plan.kept_answers(),
        purge: purge_folders(plan, &roots),
        roots: roots.used(&used),
        resources,
    };
    // the receipt changes inside the transaction too: a crash before the
    // commit mark puts back the old one with the old files.
    if let Err(e) = run.commit(&receipt) {
        let undos = std::mem::take(&mut run.undos);
        drop(journal);
        return Err(undone(&e, &undos, platform, store, &plan.package));
    }
    drop(journal);
    // committed. a backup still held open (the old exe of a running app)
    // keeps the journal around; the next kago run finishes the cleanup.
    let _ = cleanup(store, &plan.package);

    if let Some(previous) = previous {
        drop_stale(&previous, &receipt, platform);
    }
    Ok(receipt)
}

/// takes back every step after `e` stopped the install: the error to
/// return, saying whether everything came back.
fn undone(
    e: &Error,
    undos: &[Undo],
    platform: &dyn Platform,
    store: &Store,
    package: &crate::package::PackageId,
) -> Error {
    let failures = roll_back(undos, platform);
    if !failures.is_empty() {
        return Error::Other(format!(
            "{e}\nrollback was incomplete; run the setup or uninstall again to finish it:\n  {}",
            failures.join("\n  ")
        ));
    }
    match cleanup(store, package) {
        Ok(()) => Error::RolledBack(e.to_string()),
        Err(cleanup) => cleanup,
    }
}

/// where the app's own data folders the Kagofile names are, for purging.
fn purge_folders(plan: &InstallPlan, roots: &Roots) -> Vec<PathBuf> {
    plan.purge
        .iter()
        .filter_map(|text| {
            crate::install::location(text, roots, &|name| std::env::var(name).ok()).ok()
        })
        .collect()
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
                Need::Software { detect } => platform.satisfied(&detect.check),
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
        | (Resource::FirewallRule { name: a, .. }, Resource::FirewallRule { name: b, .. })
        | (Resource::Service { name: a }, Resource::Service { name: b })
        | (Resource::LoginItem { name: a }, Resource::LoginItem { name: b }) => a == b,
        (Resource::Shortcut { path: a }, Resource::Shortcut { path: b }) => a == b,
        // a value set is the same wherever it is set; an added item is
        // only the same item.
        (
            Resource::Changed {
                slot: old_slot,
                edit: old_edit,
                value: old_value,
                ..
            },
            Resource::Changed {
                slot, edit, value, ..
            },
        ) => old_slot == slot && old_edit == edit && (*edit != Edit::Add || old_value == value),
        (
            Resource::UninstallHook {
                command: a,
                arguments: x,
                ..
            },
            Resource::UninstallHook {
                command: b,
                arguments: y,
                ..
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

    /// writes `receipt` and the commit mark, with the old receipt kept
    /// aside in the journal until then.
    fn commit(&mut self, receipt: &Receipt) -> Result<(), Error> {
        let file = self.store.receipt_path(&self.plan.package);
        let undo = if file.is_file() {
            let backup = self
                .store
                .backup_dir(&self.plan.package)
                .join(&self.transaction)
                .join("receipt.json");
            let dir = backup.parent().expect("backup has a parent");
            fs::create_dir_all(dir).map_err(|e| Error::io(dir, &e))?;
            fs::copy(&file, &backup).map_err(|e| Error::io(&backup, &e))?;
            Undo::RestoreFile { path: file, backup }
        } else {
            Undo::RemoveFile { path: file }
        };
        self.step(undo)?;
        self.store.write_receipt(receipt)?;
        self.journal.write(&Entry::Commit)
    }

    fn step(&mut self, undo: Undo) -> Result<(), Error> {
        self.journal.step(undo.clone())?;
        self.undos.push(undo);
        Ok(())
    }

    fn resolve(&self, path: &SemanticPath) -> Result<PathBuf, Error> {
        self.roots.resolve(path).map_err(Error::Other)
    }

    /// [`resolve`](Self::resolve), for writing there: never through a link
    /// another user could have made; see [`super::linked`].
    fn real(&self, path: &SemanticPath) -> Result<PathBuf, Error> {
        let dest = self.resolve(path)?;
        match super::linked(self.plan.scope, &self.roots.0, path) {
            Some(link) => Err(Error::Other(format!(
                "{} is a link to somewhere else; an install for everyone only writes to real folders there",
                link.display()
            ))),
            None => Ok(dest),
        }
    }

    fn op(&mut self, op: &Operation) -> Result<(), Error> {
        match op {
            Operation::CreateDirectory { path } => self.directory(path),
            Operation::InstallFile { .. } => self.file(op),
            Operation::AddPathEntry { directory, scope } => self.path_entry(directory, *scope),
            Operation::Run {
                phase,
                command,
                undo,
                required,
            } => self.run(*phase, command, undo.as_ref(), *required),
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
                scope: Scope::User,
                ..
            } => self.login_item(id, command, arguments),
            Operation::InstallService {
                scope: Scope::Machine,
                ..
            } => self.windows_service(op),
            Operation::AllowThroughFirewall { .. } => self.firewall(op),
            Operation::Change { place, edit, value } => self.change(place, *edit, value),
        }
    }

    /// one of the package's files, put where the plan says.
    fn file(&mut self, op: &Operation) -> Result<(), Error> {
        let Operation::InstallFile {
            source,
            target,
            size,
            sha256,
            ownership,
        } = op
        else {
            return Ok(());
        };
        let dest = self.real(target)?;
        let earlier = self.previous.and_then(|p| {
            p.resources.iter().find_map(|r| match r {
                Resource::File { path, replaced, .. } if path == target => Some(replaced.clone()),
                _ => None,
            })
        });
        if let Some(kept) = self.keep_user_file(target, &dest, *ownership) {
            self.resources.push(Resource::File {
                path: target.clone(),
                size: *size,
                sha256: kept,
                ownership: *ownership,
                replaced: earlier.flatten(),
            });
            return Ok(());
        }
        // a file of the person's own outside the app's folder, which
        // uninstall would otherwise take with it, is kept aside.
        let theirs = earlier.is_none()
            && dest.is_file()
            && matches!(ownership, Ownership::Owned | Ownership::Generated)
            && !matches!(target.root, Root::App | Root::Bin);
        let aside = theirs.then(|| {
            self.store
                .replaced_dir(&self.plan.package)
                .join(format!("{}-{}", self.transaction, self.backups))
        });
        let replaced = aside.clone().or_else(|| earlier.flatten());
        self.place_aside(&dest, aside, |dest| {
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
            replaced,
        });
        Ok(())
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
    ) -> Result<(), Error> {
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

    /// a Windows service, named after the package so two never clash.
    fn windows_service(&mut self, op: &Operation) -> Result<(), Error> {
        let Operation::InstallService {
            id,
            command,
            arguments,
            name,
            manual,
            restart,
            ..
        } = op
        else {
            return Ok(());
        };
        let exe = self.resolve(command)?;
        let service = crate::platform::WindowsService {
            display_name: name
                .clone()
                .unwrap_or_else(|| format!("{} {id}", self.plan.name)),
            command_line: crate::platform::command_line(&exe, arguments),
            automatic: !manual,
            restart_on_failure: *restart,
        };
        let service = &service;
        let name = format!("{}.{id}", self.plan.package);
        let previous = self.platform.service(&name).map_err(Error::Other)?;
        self.step(Undo::RestoreService {
            name: name.clone(),
            previous,
        })?;
        self.platform
            .set_service(&name, Some(service))
            .map_err(Error::Other)?;
        self.resources.push(Resource::Service { name });
        Ok(())
    }

    /// sets a value outside the package and owns it, so uninstall can
    /// put back what was there. an update keeps what was there before
    /// the first install, not the older version's value.
    fn change(&mut self, place: &Place, edit: Edit, value: &Value) -> Result<(), Error> {
        let slot = crate::install::slot(place, self.plan.scope, self.roots, &|name| {
            std::env::var(name).ok()
        })
        .map_err(Error::Other)?;
        let value = crate::install::fill(value, self.roots);
        let earlier = self.previous.and_then(|p| {
            p.resources.iter().find_map(|r| match r {
                Resource::Changed {
                    slot: s,
                    edit: e,
                    value: v,
                    previous,
                } if *s == slot && *e == edit && (edit != Edit::Add || *v == value) => {
                    Some(previous.clone())
                }
                _ => None,
            })
        });
        if edit == Edit::Add {
            let added = self
                .platform
                .add_item(&slot, &value)
                .map_err(Error::Other)?;
            if added {
                self.step(Undo::TakeOut {
                    slot: slot.clone(),
                    item: value.clone(),
                })?;
            }
            if added || earlier.is_some() {
                self.resources.push(Resource::Changed {
                    slot,
                    edit,
                    value,
                    previous: None,
                });
            }
            return Ok(());
        }
        let now = self.platform.value(&slot).map_err(Error::Other)?;
        if edit == Edit::Init && now.is_some() {
            return Ok(());
        }
        if now.as_ref() != Some(&value) {
            self.step(Undo::PutBack {
                slot: slot.clone(),
                previous: now.clone(),
            })?;
            self.platform
                .set_value(&slot, Some(&value))
                .map_err(Error::Other)?;
        } else if earlier.is_none() {
            // someone else set the same value; it stays theirs.
            return Ok(());
        }
        if edit == Edit::Set {
            self.resources.push(Resource::Changed {
                slot,
                edit,
                value,
                previous: earlier.unwrap_or(now),
            });
        }
        Ok(())
    }

    /// a firewall rule for one of the package's programs. per user there is
    /// no right to add one; Windows asks when the program first listens.
    fn firewall(&mut self, op: &Operation) -> Result<(), Error> {
        let Operation::AllowThroughFirewall {
            name,
            program,
            public,
            protocol,
            ports,
        } = op
        else {
            return Ok(());
        };
        if self.plan.scope == Scope::User {
            return Ok(());
        }
        let rule = crate::platform::Firewall {
            program: self.resolve(program)?,
            public: *public,
            protocol: *protocol,
            ports: ports.clone(),
        };
        let group = self.plan.package.to_string();
        let previous = self
            .platform
            .firewall_rule(name, &group)
            .map_err(Error::Other)?;
        self.step(Undo::RestoreFirewallRule {
            name: name.to_owned(),
            group: group.clone(),
            previous,
        })?;
        self.platform
            .set_firewall_rule(name, &group, Some(&rule))
            .map_err(Error::Other)?;
        self.resources.push(Resource::FirewallRule {
            name: name.to_owned(),
            group,
        });
        Ok(())
    }

    /// a program for `install` runs now, and a failing one rolls the
    /// install back; its `undo` runs on rollback and at uninstall. one for
    /// `uninstall` is kept for then; a `required` one can stop it.
    fn run(
        &mut self,
        phase: RunAt,
        command: &crate::install::Invocation,
        undo: Option<&crate::install::Invocation>,
        required: bool,
    ) -> Result<(), Error> {
        let (exe, arguments) = self.invocation(command)?;
        if phase == RunAt::Uninstall {
            self.resources.push(Resource::UninstallHook {
                command: exe.display().to_string(),
                arguments,
                required,
            });
            return Ok(());
        }
        super::program::run(&exe, &arguments).map_err(Error::Other)?;
        if let Some(undo) = undo {
            let (exe, arguments) = self.invocation(undo)?;
            self.step(Undo::Run {
                command: exe.clone(),
                arguments: arguments.clone(),
            })?;
            self.resources.push(Resource::UninstallHook {
                command: exe.display().to_string(),
                arguments,
                required: false,
            });
        }
        Ok(())
    }

    /// the program to start and its arguments, with the roots filled in.
    fn invocation(
        &self,
        invocation: &crate::install::Invocation,
    ) -> Result<(PathBuf, Vec<String>), Error> {
        let exe = match &invocation.program {
            crate::install::Program::Installed { path } => self.resolve(path)?,
            crate::install::Program::System { name } => PathBuf::from(name),
        };
        let arguments = invocation
            .arguments
            .iter()
            .map(|a| crate::install::fill_text(a, self.roots))
            .collect();
        Ok((exe, arguments))
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
        let dir = self.real(path)?;
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
        self.place_aside(dest, None, write)
    }

    /// [`place`](Self::place), moving an existing file to `aside` when
    /// given, where it outlives the transaction.
    fn place_aside(
        &mut self,
        dest: &Path,
        aside: Option<PathBuf>,
        write: impl FnOnce(&Path) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if dest.exists() {
            let backup = aside.unwrap_or_else(|| {
                self.store
                    .backup_dir(&self.plan.package)
                    .join(&self.transaction)
                    .join(self.backups.to_string())
            });
            self.backups += 1;
            let dir = backup.parent().expect("backup has a parent");
            fs::create_dir_all(dir).map_err(|e| Error::io(dir, &e))?;
            self.step(Undo::RestoreFile {
                path: dest.to_path_buf(),
                backup: backup.clone(),
            })?;
            move_file(dest, &backup).map_err(|e| Error::io(dest, &e))?;
        } else {
            self.step(Undo::RemoveFile {
                path: dest.to_path_buf(),
            })?;
        }
        write(dest)
    }

    /// `{app}/uninstall.exe`: the runtime as a window program, like NSIS's
    /// uninstaller. owned like any other file, so updates replace it and
    /// uninstall removes it.
    fn uninstaller(&mut self) -> Result<PathBuf, Error> {
        let path: SemanticPath = format!("{{app}}/uninstall{}", std::env::consts::EXE_SUFFIX)
            .parse()
            .map_err(|e: crate::manifest::PathError| Error::Other(e.to_string()))?;
        let dest = self.resolve(&path)?;
        // a signed uninstaller staged next to the runtime goes in as it
        // is: patching a signed file would break its signature.
        let signed = self.runtime.with_file_name(super::UNINSTALLER);
        let mut bytes = if signed.is_file() {
            fs::read(&signed).map_err(|e| Error::io(&signed, &e))?
        } else {
            fs::read(self.runtime).map_err(|e| Error::io(self.runtime, &e))?
        };
        crate::format::setup::make_windowed(&mut bytes);
        self.place(&dest, |dest| {
            fs::write(dest, &bytes).map_err(|e| Error::io(dest, &e))
        })?;
        let (size, sha256) = hash(&dest).map_err(|e| Error::io(&dest, &e))?;
        self.resources.push(Resource::File {
            path,
            size,
            sha256,
            ownership: Ownership::Owned,
            replaced: None,
        });
        Ok(dest)
    }

    fn register(
        &mut self,
        display_name: &str,
        entrypoint: Option<&SemanticPath>,
        icon: Option<&SemanticPath>,
    ) -> Result<(), Error> {
        let runtime = self.store.runtime_path();
        copy_runtime(self.runtime, &runtime)?;
        let uninstaller = self.uninstaller()?;
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
            uninstaller: Some(uninstaller),
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
