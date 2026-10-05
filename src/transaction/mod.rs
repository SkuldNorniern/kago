//! durable install, uninstall and crash recovery.
//!
//! every mutating step writes its undo to a synced journal first. a
//! failure rolls back in-process; a crash is rolled back by the next kago
//! run. the receipt is written only once everything applied.

mod apply;
mod hook;
mod journal;
mod recovery;
mod store;
mod verify;

use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

pub use journal::{Entry, Undo};
pub use recovery::Recovered;
pub use store::Store;

/// file name of a ready, signed uninstaller staged next to the runtime.
pub const UNINSTALLER: &str = "uninstall-signed.exe";
pub use verify::{Verified, verify};

use crate::install::{InstallPlan, Operation, Scope, hash};
use crate::manifest::Ownership;
use crate::package::PackageId;
use crate::platform::Platform;
use crate::receipt::{Receipt, Resource};

#[derive(Debug)]
pub enum Error {
    /// another kago is working on this package.
    Locked(PackageId),
    NotInstalled(PackageId),
    /// a newer version is installed and the package denies downgrades.
    Downgrade {
        installed: String,
        offered: String,
    },
    /// the machine lacks what the package needs; nothing was changed.
    Unmet(Vec<String>),
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
            Self::Downgrade { installed, offered } => write!(
                f,
                "{installed} is installed, which is newer than {offered}; downgrades are not allowed for this package"
            ),
            Self::Unmet(needs) => {
                write!(
                    f,
                    "this package needs {}; nothing was changed",
                    needs.join(", ")
                )
            }
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
    install_observed(plan, platform, store, runtime, &mut |_, _, _| {})
}

/// [`install`], telling `observe` about each operation before it runs.
///
/// # Errors
///
/// as [`install`].
pub fn install_observed(
    plan: &InstallPlan,
    platform: &dyn Platform,
    store: &Store,
    runtime: &Path,
    observe: &mut dyn FnMut(usize, usize, &Operation),
) -> Result<Receipt, Error> {
    let _lock = lock(store, &plan.package)?;
    recovery::recover_one(store, &plan.package, platform)?;
    apply::install(plan, platform, store, runtime, observe)
}

/// what uninstall left behind on purpose or could not remove.
#[derive(Debug, Default)]
pub struct Uninstalled {
    /// user data and modified files kept by ownership rules.
    pub kept: Vec<String>,
    pub failed: Vec<String>,
    /// in use, so removed when the computer next starts; see
    /// [`remove_at_restart`].
    pub at_restart: Vec<String>,
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
        at_restart: Vec::new(),
    })
}

/// what an earlier uninstall could not remove because it was in use:
/// deleted when the computer next starts, like NSIS's `/REBOOTOK`, and no
/// longer the package's. returns those paths; what cannot be scheduled
/// stays in the receipt.
///
/// # Errors
///
/// when locked or the receipt cannot be read or written.
pub fn remove_at_restart(
    package: &PackageId,
    platform: &dyn Platform,
    store: &Store,
) -> Result<Vec<String>, Error> {
    let held = lock(store, package)?;
    let Some(mut receipt) = store.read_receipt(package)? else {
        return Ok(Vec::new());
    };
    let mut scheduled = Vec::new();
    let mut left = Vec::new();
    // Windows deletes them in the order asked: files, then folders
    // deepest first, so each folder is empty by its turn.
    let mut resources = std::mem::take(&mut receipt.resources);
    resources.sort_by_key(|r| match r {
        Resource::Directory { path } => usize::MAX - path.segments().count(),
        _ => 0,
    });
    for resource in resources {
        let path = match &resource {
            Resource::File { path, .. } | Resource::Directory { path } => receipt.absolute(path),
            Resource::Shortcut { path } => Some(path.clone()),
            _ => None,
        };
        match path {
            Some(path) if !path.exists() => {}
            Some(path) if platform.delete_at_restart(&path).is_ok() => {
                scheduled.push(path.display().to_string());
            }
            _ => left.push(resource),
        }
    }
    if left.is_empty() {
        store.remove_receipt(package)?;
        drop(held);
        let _ = fs::remove_file(store.lock_path(package));
    } else {
        receipt.resources = left;
        store.write_receipt(&receipt)?;
    }
    Ok(scheduled)
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
        // the package's own cleanup runs while its files are still there.
        Resource::UninstallHook { .. } => 0,
        Resource::Registration { .. } => 1,
        Resource::Shortcut { .. } => 2,
        Resource::PathEntry { .. }
        | Resource::SteamVrDriver { .. }
        | Resource::LoginItem { .. } => 3,
        Resource::File { .. } => 4,
        Resource::Directory { path } => 5 + usize::MAX / 2 - path.segments().count(),
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
            Resource::UninstallHook { command, arguments } => {
                // best effort: a broken hook must not keep the package from
                // being removed.
                let _ = hook::run(Path::new(command), arguments);
                Ok(())
            }
            Resource::LoginItem { name } => platform.set_login_item(name, None),
            Resource::SteamVrDriver { directory } => {
                platform.remove_steamvr_driver(Path::new(directory))
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
