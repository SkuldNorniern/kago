use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Error;
use crate::install::Scope;
use crate::package::{PackageId, Version};

/// how to take back one step. written before the step runs, so every undo
/// must be fine when the step never happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "undo", rename_all = "kebab-case")]
pub enum Undo {
    RemoveFile {
        path: PathBuf,
    },
    RestoreFile {
        path: PathBuf,
        backup: PathBuf,
    },
    /// only if empty.
    RemoveDirectory {
        path: PathBuf,
    },
    RemovePathEntry {
        entry: PathBuf,
        scope: Scope,
    },
    RemoveRegistration {
        key: String,
        scope: Scope,
    },
    /// puts a value back as it was, removing it for `None`.
    PutBack {
        slot: crate::install::Slot,
        previous: Option<crate::manifest::Value>,
    },
    /// takes an added item out of its list again.
    TakeOut {
        slot: crate::install::Slot,
        item: crate::manifest::Value,
    },
    /// runs a program's `undo`, taking back what it did.
    Run {
        command: PathBuf,
        arguments: Vec<String>,
    },
    /// puts a login item back as it was, removing it for `None`.
    RestoreLoginItem {
        name: String,
        previous: Option<String>,
    },
    /// puts a Windows service back as it was, removing it for `None`.
    RestoreService {
        name: String,
        previous: Option<crate::platform::WindowsService>,
    },
    /// puts a firewall rule back as it was, removing it for `None`.
    RestoreFirewallRule {
        name: String,
        group: String,
        previous: Option<crate::platform::Firewall>,
    },
    /// takes a driver package the install added out of the driver store.
    RemoveDriver {
        published: String,
    },
    /// removes a device the install made.
    RemoveDevice {
        instance: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "kebab-case")]
pub enum Entry {
    Begin {
        package: PackageId,
        version: Version,
    },
    Step(Undo),
    /// receipt is written; leftovers only need cleanup.
    Commit,
}

/// append-only, synced after every entry.
pub struct Journal {
    path: PathBuf,
    file: fs::File,
}

impl Journal {
    pub fn begin(path: &Path, package: &PackageId, version: &Version) -> Result<Self, Error> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| Error::io(dir, &e))?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .map_err(|e| Error::io(path, &e))?;
        let mut journal = Self {
            path: path.to_path_buf(),
            file,
        };
        journal.write(&Entry::Begin {
            package: package.clone(),
            version: version.clone(),
        })?;
        Ok(journal)
    }

    pub fn write(&mut self, entry: &Entry) -> Result<(), Error> {
        let mut line = serde_json::to_string(entry).expect("journal entry serializes");
        line.push('\n');
        self.file
            .write_all(line.as_bytes())
            .and_then(|()| self.file.sync_data())
            .map_err(|e| Error::io(&self.path, &e))
    }

    pub fn step(&mut self, undo: Undo) -> Result<(), Error> {
        self.write(&Entry::Step(undo))
    }
}

/// every complete entry in a journal. a torn last line from a crash is
/// dropped, even one cut inside a character.
pub fn read(path: &Path) -> Result<Vec<Entry>, Error> {
    let bytes = fs::read(path).map_err(|e| Error::io(path, &e))?;
    let mut out = Vec::new();
    for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        match serde_json::from_slice(line) {
            Ok(entry) => out.push(entry),
            Err(_) => break,
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_torn_tail() {
        let dir = std::env::temp_dir().join(format!("kago-journal-{}", std::process::id()));
        let path = dir.join("x.journal");
        let id = PackageId::new("dev.test.app").unwrap();
        let mut j = Journal::begin(&path, &id, &Version::new(1, 0, 0)).unwrap();
        j.step(Undo::RemoveFile {
            path: PathBuf::from("a"),
        })
        .unwrap();
        drop(j);
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"entry\":\"st")
            .unwrap();
        let entries = read(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[1], Entry::Step(Undo::RemoveFile { .. })));
        // cut inside a character of a path like `C:\Users\스쿨드`.
        let whole = fs::read(&path).unwrap();
        let mut torn = whole[..whole.len() - 12].to_vec();
        torn.extend_from_slice(
            b"\n{\"entry\":\"step\",\"undo\":\"remove-file\",\"path\":\"\xec\x8a",
        );
        fs::write(&path, torn).unwrap();
        assert_eq!(read(&path).unwrap().len(), 2);
        fs::remove_dir_all(dir).unwrap();
    }
}
