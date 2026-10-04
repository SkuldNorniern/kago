use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::Error;
use crate::package::PackageId;
use crate::receipt::Receipt;

/// kago's state for one scope.
///
/// ```text
/// receipts/<package>.json
/// transactions/<package>.journal
/// transactions/<package>.backup/
/// locks/<package>.lock
/// bin/kago[.exe]                 runtime used by uninstall entries
/// ```
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn receipt_path(&self, package: &PackageId) -> PathBuf {
        self.root.join("receipts").join(format!("{package}.json"))
    }

    pub(super) fn journal_path(&self, package: &PackageId) -> PathBuf {
        self.root
            .join("transactions")
            .join(format!("{package}.journal"))
    }

    pub(super) fn backup_dir(&self, package: &PackageId) -> PathBuf {
        self.root
            .join("transactions")
            .join(format!("{package}.backup"))
    }

    pub(super) fn lock_path(&self, package: &PackageId) -> PathBuf {
        self.root.join("locks").join(format!("{package}.lock"))
    }

    /// where the shared kago runtime is copied for uninstall entries.
    #[must_use]
    pub fn runtime_path(&self) -> PathBuf {
        self.root
            .join("bin")
            .join(format!("kago{}", std::env::consts::EXE_SUFFIX))
    }

    /// # Errors
    ///
    /// when the receipt exists but cannot be read or parsed.
    pub fn read_receipt(&self, package: &PackageId) -> Result<Option<Receipt>, Error> {
        let path = self.receipt_path(package);
        let json = match fs::read_to_string(&path) {
            Ok(json) => json,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::io(&path, &e)),
        };
        Receipt::from_json(&json)
            .map(Some)
            .map_err(|e| Error::Other(format!("receipt {}: {e}", path.display())))
    }

    /// every installed package in this scope.
    ///
    /// # Errors
    ///
    /// when the receipts directory cannot be listed.
    pub fn receipts(&self) -> Result<Vec<Receipt>, Error> {
        let dir = self.root.join("receipts");
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::io(&dir, &e)),
        };
        let mut out = Vec::new();
        for entry in entries {
            let path = entry.map_err(|e| Error::io(&dir, &e))?.path();
            if path.extension().is_some_and(|e| e == "json") {
                let json = fs::read_to_string(&path).map_err(|e| Error::io(&path, &e))?;
                out.push(
                    Receipt::from_json(&json)
                        .map_err(|e| Error::Other(format!("receipt {}: {e}", path.display())))?,
                );
            }
        }
        out.sort_by(|a, b| a.package.cmp(&b.package));
        Ok(out)
    }

    /// writes next to the receipt, syncs, then renames over it.
    pub(super) fn write_receipt(&self, receipt: &Receipt) -> Result<(), Error> {
        let path = self.receipt_path(&receipt.package);
        write_atomic(&path, receipt.to_json().as_bytes())
    }

    pub(super) fn remove_receipt(&self, package: &PackageId) -> Result<(), Error> {
        let path = self.receipt_path(package);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::io(&path, &e)),
        }
    }

    /// package ids with a journal left behind.
    pub(super) fn journals(&self) -> Result<Vec<PackageId>, Error> {
        let dir = self.root.join("transactions");
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::io(&dir, &e)),
        };
        let mut out = Vec::new();
        for entry in entries {
            let path = entry.map_err(|e| Error::io(&dir, &e))?.path();
            if path.extension().is_some_and(|e| e == "journal")
                && let Some(id) = path.file_stem().and_then(|s| s.to_str())
                && let Ok(id) = PackageId::new(id)
            {
                out.push(id);
            }
        }
        out.sort();
        Ok(out)
    }
}

/// `bytes` to `path` through a synced temp file and a rename.
pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(|e| Error::io(dir, &e))?;
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp).map_err(|e| Error::io(&tmp, &e))?;
    file.write_all(bytes).map_err(|e| Error::io(&tmp, &e))?;
    file.sync_all().map_err(|e| Error::io(&tmp, &e))?;
    drop(file);
    fs::rename(&tmp, path).map_err(|e| Error::io(path, &e))
}
