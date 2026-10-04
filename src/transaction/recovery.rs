use std::fs;
use std::io::ErrorKind;

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
pub(super) fn roll_back(undos: &[Undo], platform: &dyn Platform) -> Vec<String> {
    let mut failures = Vec::new();
    for undo in undos.iter().rev() {
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
            fs::rename(backup, path).map_err(|e| format!("cannot restore {}: {e}", path.display()))
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
    }
}

/// finishes or rolls back a journal left by a crash.
pub(super) fn recover_one(
    store: &Store,
    package: &PackageId,
    platform: &dyn Platform,
) -> Result<Option<Recovered>, Error> {
    let path = store.journal_path(package);
    if !path.exists() {
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
