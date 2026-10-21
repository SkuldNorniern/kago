//! durable install, uninstall and crash recovery.
//!
//! every mutating step writes its undo to a synced journal first. a
//! failure rolls back in-process; a crash is rolled back by the next kago
//! run. the receipt is written only once everything applied.

mod apply;
mod journal;
mod program;
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
    /// a `required` program run at uninstall failed; nothing was removed.
    Kept(String),
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
            Self::Kept(e) => write!(f, "{e}\nthe uninstall was stopped, nothing was removed"),
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
/// when the platform has no state location for the scope, or an elevated
/// kago cannot make the one for everyone writable by administrators only.
pub fn store(platform: &dyn Platform, scope: Scope) -> Result<Store, Error> {
    let dir = platform.state_dir(scope).map_err(Error::Other)?;
    // what an elevated kago trusts must not be writable by every user.
    if scope == Scope::Machine {
        platform.secure_state(&dir).map_err(Error::Other)?;
    }
    Ok(Store::new(dir))
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
/// when locked or not installed, or a `required` program says no.
pub fn uninstall(
    package: &PackageId,
    platform: &dyn Platform,
    store: &Store,
) -> Result<Uninstalled, Error> {
    ask(package, store)?;
    remove(package, platform, store)
}

/// runs the package's `required` programs for uninstall, which may say no:
/// the first that fails stops the uninstall before anything is closed or
/// removed. [`remove`] does not run them again.
///
/// # Errors
///
/// [`Error::Kept`] when one fails; when locked, not installed or the
/// receipt cannot be read.
pub fn ask(package: &PackageId, store: &Store) -> Result<(), Error> {
    let held = lock(store, package)?;
    let receipt = store
        .read_receipt(package)?
        .ok_or_else(|| Error::NotInstalled(package.clone()))?;
    // the program may wait for a person; the package is not held meanwhile.
    drop(held);
    for resource in &receipt.resources {
        if let Resource::UninstallHook {
            command,
            arguments,
            required: true,
        } = resource
        {
            program::run_waiting(Path::new(command), arguments).map_err(Error::Kept)?;
        }
    }
    Ok(())
}

/// removes what [`uninstall`] removes, without asking first.
///
/// # Errors
///
/// when locked or not installed.
pub fn remove(
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
        let _ = fs::remove_dir(store.replaced_dir(package));
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

/// after an uninstall, what it kept on purpose too: the files it kept
/// (user data, changed settings) and the package's own settings and data
/// folders, with whatever the app put there. a folder only goes when it
/// is named after the package, so a root set wrong never takes more.
/// returns what could not be removed.
#[must_use]
pub fn purge(receipt: &Receipt, platform: &dyn Platform, kept: &[String]) -> Vec<String> {
    use crate::manifest::Root;
    let mut failed = Vec::new();
    // only the person's own: never a shared or external file another
    // program may use.
    let theirs: Vec<std::path::PathBuf> = receipt
        .resources
        .iter()
        .filter_map(|r| match r {
            Resource::File {
                path,
                ownership: Ownership::UserData | Ownership::PreserveIfModified,
                ..
            } => receipt.absolute(path),
            _ => None,
        })
        .collect();
    for path in theirs.iter().filter(|p| p.is_file()) {
        if let Err(e) = fs::remove_file(path) {
            failed.push(format!("{}: {e}", path.display()));
        }
    }
    let folder = crate::install::folder_name(&receipt.name);
    let app = receipt.roots.get(&Root::App);
    let mut dirs: Vec<std::path::PathBuf> = platform
        .roots(receipt.scope, &receipt.name)
        .map(|roots| {
            [
                Root::Config,
                Root::Data,
                Root::Cache,
                Root::Logs,
                Root::State,
            ]
            .iter()
            .filter_map(|r| roots.0.get(r).cloned())
            .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs.dedup();
    for dir in dirs {
        let named = dir
            .components()
            .any(|c| c.as_os_str().to_str() == Some(folder.as_str()));
        let lower = |p: &Path| p.display().to_string().to_lowercase();
        let inside_app = app.is_some_and(|a| {
            let (a, d) = (lower(a), lower(&dir));
            a == d
                || a.starts_with(&format!("{d}{}", std::path::MAIN_SEPARATOR))
                || d.starts_with(&format!("{a}{}", std::path::MAIN_SEPARATOR))
        });
        if !named || inside_app || !dir.exists() || platform.shared_folder(&dir) {
            continue;
        }
        if let Err(e) = fs::remove_dir_all(&dir) {
            failed.push(format!("{}: {e}", dir.display()));
        }
    }
    // the app's own data folders the Kagofile names with `purge`.
    for dir in receipt.purge.iter().filter(|d| d.exists()) {
        if platform.shared_folder(dir) {
            failed.push(format!(
                "{}: other programs use this folder too, so it was kept",
                dir.display()
            ));
            continue;
        }
        let removed = if dir.is_dir() {
            fs::remove_dir_all(dir)
        } else {
            fs::remove_file(dir)
        };
        if let Err(e) = removed {
            failed.push(format!("{}: {e}", dir.display()));
        }
    }
    // folders that only held kept files.
    for path in kept.iter().map(Path::new).filter(|p| p.is_dir()) {
        let _ = fs::remove_dir(path);
    }
    failed
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
    // rest goes at restart, so the entry goes now.
    if left
        .iter()
        .all(|r| matches!(r, Resource::Registration { .. }))
    {
        left.retain(|r| match r {
            Resource::Registration { key, scope } => platform.unregister(*scope, key).is_err(),
            _ => true,
        });
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

/// removes `resources` of `receipt`: shortcuts and PATH first, then files,
/// then directories deepest first and only when empty. uninstaller and
/// registration go last, only when nothing failed, so uninstall can rerun.
fn remove_resources(receipt: &Receipt, resources: &[Resource], platform: &dyn Platform) -> Removal {
    let mut out = Removal {
        left: Vec::new(),
        kept: Vec::new(),
        failed: Vec::new(),
    };
    let uninstaller = format!("{{app}}/uninstall{}", std::env::consts::EXE_SUFFIX);
    let is_uninstaller =
        |r: &Resource| matches!(r, Resource::File { path, .. } if path.to_string() == uninstaller);
    let rank = |r: &Resource| match r {
        // the package's own cleanup runs while its files are still there.
        Resource::UninstallHook { .. } => 0,
        Resource::Shortcut { .. } => 2,
        Resource::PathEntry { .. }
        | Resource::LoginItem { .. }
        | Resource::FirewallRule { .. }
        | Resource::Service { .. }
        | Resource::Changed { .. } => 3,
        Resource::File { .. } if is_uninstaller(r) => 5,
        Resource::File { .. } => 4,
        Resource::Directory { path } => 6 + usize::MAX / 2 - path.segments().count(),
        Resource::Registration { .. } => usize::MAX,
    };
    let mut ordered: Vec<&Resource> = resources.iter().collect();
    ordered.sort_by_key(|r| rank(r));
    for resource in ordered {
        let last = is_uninstaller(resource)
            || matches!(
                resource,
                Resource::Registration { .. } | Resource::Directory { .. }
            );
        if last && !out.failed.is_empty() {
            out.left.push(resource.clone());
            continue;
        }
        let result = match resource {
            Resource::Registration { key, scope } => platform.unregister(*scope, key),
            Resource::Shortcut { path } => remove_file(path),
            Resource::PathEntry { entry, scope } => {
                platform.remove_path_entry(*scope, Path::new(entry))
            }
            // a required one already ran in `ask`.
            Resource::UninstallHook { required: true, .. } => Ok(()),
            Resource::UninstallHook {
                command, arguments, ..
            } => {
                // best effort: a broken hook must not keep the package from
                // being removed.
                let _ = program::run(Path::new(command), arguments);
                Ok(())
            }
            Resource::LoginItem { name } => platform.set_login_item(name, None),
            Resource::FirewallRule { name, group } => platform.set_firewall_rule(name, group, None),
            Resource::Service { name } => platform.set_service(name, None),
            Resource::Changed {
                slot,
                edit,
                value,
                previous,
            } => revert(platform, slot, *edit, value, previous.as_ref()),
            Resource::File { .. } => match remove_owned_file(receipt, resource) {
                Ok(Some(kept)) => {
                    out.kept.push(kept);
                    continue;
                }
                Ok(None) => Ok(()),
                Err(e) => Err(e),
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

/// takes back a change at uninstall: an added item comes out; a value
/// set goes back to what it was, unless someone changed it since.
fn revert(
    platform: &dyn Platform,
    slot: &crate::install::Slot,
    edit: crate::manifest::Edit,
    value: &crate::manifest::Value,
    previous: Option<&crate::manifest::Value>,
) -> Result<(), String> {
    use crate::manifest::Edit;
    match edit {
        Edit::Add => platform.remove_item(slot, value),
        Edit::Set => {
            if platform.value(slot)?.as_ref() == Some(value) {
                platform.set_value(slot, previous)
            } else {
                Ok(())
            }
        }
        Edit::Init => Ok(()),
    }
}

/// removes one file of the package, unless ownership keeps it: then
/// where it is, to report as kept. the person's own file it replaced
/// goes back.
fn remove_owned_file(receipt: &Receipt, file: &Resource) -> Result<Option<String>, String> {
    let Resource::File {
        path,
        sha256,
        ownership,
        replaced,
        ..
    } = file
    else {
        return Ok(None);
    };
    let abs = receipt
        .absolute(path)
        .ok_or_else(|| format!("{path}: root missing from receipt"))?;
    let keep = match ownership {
        Ownership::Owned | Ownership::Generated => false,
        Ownership::PreserveIfModified => hash(&abs).is_ok_and(|(_, now)| now != *sha256),
        Ownership::UserData | Ownership::Shared | Ownership::External => true,
    };
    if keep {
        return Ok(Some(abs.display().to_string()));
    }
    remove_file(&abs)?;
    match replaced {
        Some(original) if original.is_file() => recovery::move_file(original, &abs)
            .map(|()| None)
            .map_err(|e| format!("cannot put back {}: {e}", abs.display())),
        _ => Ok(None),
    }
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
