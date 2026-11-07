use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::journal::{self, Entry, Undo};
use super::{Error, Store};
use crate::package::PackageId;
use crate::platform::Platform;

/// what recovery did with one leftover journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovered {
    /// commit marker was there, only cleanup was left.
    Finished(PackageId),
    RolledBack(PackageId),
}

/// takes back undo records newest first. keeps going past failures and
/// returns them all, so one stuck file does not leave the rest applied.
///
/// services stop first and come back last, on the old program.
pub(super) fn roll_back(undos: &[Undo], platform: &dyn Platform) -> Vec<String> {
    let mut failures = Vec::new();
    let is_service = |undo: &&Undo| matches!(undo, Undo::RestoreService { .. });
    for undo in undos.iter().rev().filter(is_service) {
        if let Undo::RestoreService { name, .. } = undo
            && let Err(e) = platform.stop_service(name)
        {
            failures.push(e);
        }
    }
    let (services, rest): (Vec<&Undo>, Vec<&Undo>) = undos.iter().rev().partition(is_service);
    for undo in rest.into_iter().chain(services) {
        if let Err(e) = take_back(undo, platform) {
            failures.push(e);
        }
    }
    failures
}

fn take_back(undo: &Undo, platform: &dyn Platform) -> Result<(), String> {
    let gone = |r: std::io::Result<()>, what: &str, path: &std::path::Path| match r {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot {what} {}: {e}", path.display())),
    };
    match undo {
        Undo::RemoveFile { path } => gone(fs::remove_file(path), "remove", path),
        Undo::RestoreFile { path, backup } => {
            if !backup.exists() {
                // never moved aside, nothing to restore.
                return Ok(());
            }
            gone(fs::remove_file(path), "remove", path)?;
            // folder may be gone since.
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot restore {}: {e}", path.display()))?;
            }
            move_file(backup, path).map_err(|e| format!("cannot restore {}: {e}", path.display()))
        }
        Undo::RemoveDirectory { path } => match fs::remove_dir(path) {
            Ok(()) => Ok(()),
            // not empty means something else lives there now; leave it.
            Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty) => {
                Ok(())
            }
            Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
        },
        Undo::RemovePathEntry { entry, scope } => platform.remove_path_entry(*scope, entry),
        Undo::RemoveRegistration { key, scope } => platform.unregister(*scope, key),
        Undo::Run { command, arguments } => super::program::run(command, arguments),
        Undo::PutBack { slot, previous } => platform.set_value(slot, previous.as_ref()),
        Undo::TakeOut { slot, item } => platform.remove_item(slot, item),
        Undo::RestoreService { name, previous } => platform.set_service(name, previous.as_ref()),
        Undo::RestoreFirewallRule {
            name,
            group,
            previous,
        } => platform.set_firewall_rule(name, group, previous.as_ref()),
        Undo::RemoveDriver { published } => platform.remove_driver(published),
        Undo::RemoveDevice { instance } => platform.remove_device(instance),
        Undo::RestoreLoginItem { name, previous } => {
            platform.set_login_item(name, previous.as_deref())
        }
    }
}

/// rename, or across drives copy and delete. the copy is written under
/// another name and renamed when whole, so a crash never leaves half a
/// file where rollback would take it for the original.
pub(super) fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let mut partial = to.as_os_str().to_owned();
    partial.push(".kago-partial");
    let partial = PathBuf::from(partial);
    fs::copy(from, &partial)?;
    fs::File::open(&partial)?.sync_all()?;
    fs::rename(&partial, to)?;
    fs::remove_file(from)
}

/// finishes or rolls back a journal left by a crash.
pub(super) fn recover_one(
    store: &Store,
    package: &PackageId,
    platform: &dyn Platform,
) -> Result<Option<Recovered>, Error> {
    let path = store.journal_path(package);
    if !path.exists() {
        // backups a finished run could not delete, like a running exe.
        let _ = fs::remove_dir_all(store.backup_dir(package));
        return Ok(None);
    }
    let entries = journal::read(&path)?;
    let committed = entries.iter().any(|e| matches!(e, Entry::Commit));
    let outcome = if committed {
        Recovered::Finished(package.clone())
    } else {
        let undos: Vec<Undo> = entries
            .into_iter()
            .filter_map(|e| match e {
                Entry::Step(u) => Some(u),
                _ => None,
            })
            .collect();
        let failures = roll_back(&undos, platform);
        if !failures.is_empty() {
            return Err(Error::Other(format!(
                "recovery of {package} could not undo everything:\n  {}",
                failures.join("\n  ")
            )));
        }
        Recovered::RolledBack(package.clone())
    };
    cleanup(store, package)?;
    Ok(Some(outcome))
}

/// drops the journal and, where possible, the backups of a finished
/// transaction.
pub(super) fn cleanup(store: &Store, package: &PackageId) -> Result<(), Error> {
    // a backup of a still running exe cannot go yet; a later cleanup
    // retries. its name is unique, so nothing new collides with it.
    let _ = fs::remove_dir_all(store.backup_dir(package));
    let journal = store.journal_path(package);
    match fs::remove_file(&journal) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(&journal, &e)),
    }
}
