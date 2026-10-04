//! durable install, uninstall and crash recovery.
//!
//! every mutating step writes its undo to a synced journal first. a
//! failure rolls back in-process; a crash is rolled back by the next kago
//! run. the receipt is written only once everything applied.

mod apply;
mod journal;
mod recovery;
mod store;

use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

pub use journal::{Entry, Undo};
pub use recovery::Recovered;
pub use store::Store;

use crate::install::{InstallPlan, Scope, hash};
use crate::manifest::Ownership;
use crate::package::PackageId;
use crate::platform::Platform;
use crate::receipt::{Receipt, Resource};

#[derive(Debug)]
pub enum Error {
    /// another kago is working on this package.
    Locked(PackageId),
    NotInstalled(PackageId),
    /// the install failed and everything it did was taken back.
    RolledBack(String),
    Other(String),
}

impl Error {
    pub(crate) fn io(path: &Path, e: &std::io::Error) -> Self {
        Self::Other(format!("{}: {e}", path.display()))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked(p) => write!(f, "{p} is being changed by another kago process"),
            Self::NotInstalled(p) => write!(f, "{p} is not installed in this scope"),
            Self::RolledBack(e) => {
                write!(f, "{e}\nnothing was changed, the install was rolled back")
            }
            Self::Other(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for Error {}

/// exclusive per-package lock, released on drop or process exit.
struct Lock(#[allow(dead_code)] fs::File);

fn lock(store: &Store, package: &PackageId) -> Result<Lock, Error> {
    let path = store.lock_path(package);
    let dir = path.parent().expect("lock has a parent");
    fs::create_dir_all(dir).map_err(|e| Error::io(dir, &e))?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| Error::io(&path, &e))?;
    match file.try_lock() {
        Ok(()) => Ok(Lock(file)),
        Err(fs::TryLockError::WouldBlock) => Err(Error::Locked(package.clone())),
        Err(fs::TryLockError::Error(e)) => Err(Error::io(&path, &e)),
    }
}

/// state store for `scope` on `platform`.
///
/// # Errors
///
/// when the platform has no state location for the scope.
pub fn store(platform: &dyn Platform, scope: Scope) -> Result<Store, Error> {
    platform
        .state_dir(scope)
        .map(Store::new)
        .map_err(Error::Other)
}

/// installs or upgrades. `runtime` is the kago executable that uninstall
/// entries will call; it is copied into the state store.
///
/// # Errors
///
/// when locked, when a leftover transaction cannot be recovered, or when a
/// step fails (then everything is rolled back).
pub fn install(
    plan: &InstallPlan,
    platform: &dyn Platform,
    store: &Store,
    runtime: &Path,
) -> Result<Receipt, Error> {
    let _lock = lock(store, &plan.package)?;
    recovery::recover_one(store, &plan.package, platform)?;
    apply::install(plan, platform, store, runtime)
}

/// what uninstall left behind on purpose or could not remove.
#[derive(Debug, Default)]
pub struct Uninstalled {
    /// user data and modified files kept by ownership rules.
    pub kept: Vec<String>,
    pub failed: Vec<String>,
}

/// removes everything the receipt owns, keeps what ownership says to keep.
/// when something cannot be removed the receipt stays with just those
/// resources, so uninstall can run again.
///
/// # Errors
///
/// when locked or not installed.
pub fn uninstall(
    package: &PackageId,
    platform: &dyn Platform,
    store: &Store,
) -> Result<Uninstalled, Error> {
    let held = lock(store, package)?;
    recovery::recover_one(store, package, platform)?;
    let receipt = store
        .read_receipt(package)?
        .ok_or_else(|| Error::NotInstalled(package.clone()))?;
    let outcome = remove_resources(&receipt, &receipt.resources, platform);
    if outcome.left.is_empty() {
        store.remove_receipt(package)?;
        drop(held);
        // nothing of this package is left; the lock file can go too.
        let _ = fs::remove_file(store.lock_path(package));
    } else {
        let mut rest = receipt.clone();
        rest.resources = outcome.left;
        store.write_receipt(&rest)?;
    }
    Ok(Uninstalled {
        kept: outcome.kept,
        failed: outcome.failed,
    })
}

/// finishes or rolls back every transaction a crash left in `store`.
///
/// # Errors
///
/// when a journal cannot be read or fully rolled back.
pub fn recover(platform: &dyn Platform, store: &Store) -> Result<Vec<Recovered>, Error> {
    let mut out = Vec::new();
    for package in store.journals()? {
        let _lock = lock(store, &package)?;
        if let Some(r) = recovery::recover_one(store, &package, platform)? {
            out.push(r);
        }
    }
    Ok(out)
}

struct Removal {
    /// resources that still exist and are still owned.
    left: Vec<Resource>,
    kept: Vec<String>,
    failed: Vec<String>,
}

/// removes `resources` of `receipt`: registrations, shortcuts and PATH
/// first, then files, then directories deepest first and only when empty.
fn remove_resources(receipt: &Receipt, resources: &[Resource], platform: &dyn Platform) -> Removal {
    let mut out = Removal {
        left: Vec::new(),
        kept: Vec::new(),
        failed: Vec::new(),
    };
    let rank = |r: &Resource| match r {
        Resource::Registration { .. } => 0,
        Resource::Shortcut { .. } => 1,
        Resource::PathEntry { .. } => 2,
        Resource::File { .. } => 3,
        Resource::Directory { path } => 4 + usize::MAX / 2 - path.segments().count(),
    };
    let mut ordered: Vec<&Resource> = resources.iter().collect();
    ordered.sort_by_key(|r| rank(r));
    for resource in ordered {
        let result = match resource {
            Resource::Registration { key, scope } => platform.unregister(*scope, key),
            Resource::Shortcut { path } => remove_file(path),
            Resource::PathEntry { entry, scope } => {
                platform.remove_path_entry(*scope, Path::new(entry))
            }
            Resource::File {
                path,
                sha256,
                ownership,
                ..
            } => match receipt.absolute(path) {
                None => Err(format!("{path}: root missing from receipt")),
                Some(abs) => {
                    let keep = match ownership {
                        Ownership::Owned | Ownership::Generated => false,
                        Ownership::PreserveIfModified => {
                            hash(&abs).is_ok_and(|(_, now)| now != *sha256)
                        }
                        Ownership::UserData | Ownership::Shared | Ownership::External => true,
                    };
                    if keep {
                        out.kept.push(abs.display().to_string());
                        continue;
                    }
                    remove_file(&abs)
                }
            },
            Resource::Directory { path } => match receipt.absolute(path) {
                None => Err(format!("{path}: root missing from receipt")),
                Some(abs) => match fs::remove_dir(&abs) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
                    Err(e) if e.kind() == ErrorKind::DirectoryNotEmpty => {
                        out.kept.push(abs.display().to_string());
                        continue;
                    }
                    Err(e) => Err(format!("{}: {e}", abs.display())),
                },
            },
        };
        if let Err(e) = result {
            out.failed.push(e);
            out.left.push(resource.clone());
        }
    }
    out
}

fn remove_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests;
