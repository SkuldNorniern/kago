use std::fmt;
use std::path::PathBuf;

use super::Scope;
use crate::manifest::{Ownership, SemanticPath, ShortcutLocation};

/// one step of an install. platforms carry these out; nothing here
/// touches the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    CreateDirectory {
        path: SemanticPath,
    },
    InstallFile {
        /// build-side file the payload comes from.
        source: PathBuf,
        target: SemanticPath,
        size: u64,
        sha256: Digest,
        ownership: Ownership,
    },
    AddPathEntry {
        directory: SemanticPath,
        scope: Scope,
    },
    CreateShortcut {
        name: String,
        target: SemanticPath,
        location: ShortcutLocation,
        arguments: Vec<String>,
    },
    /// uninstall entry, start menu / desktop file metadata.
    RegisterApplication {
        display_name: String,
        entrypoint: Option<SemanticPath>,
    },
    InstallService {
        id: String,
        command: SemanticPath,
        arguments: Vec<String>,
        scope: Scope,
    },
}

impl Operation {
    /// short kind name for dry-run output and logs.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::CreateDirectory { .. } => "create-directory",
            Self::InstallFile { .. } => "install-file",
            Self::AddPathEntry { .. } => "add-path-entry",
            Self::CreateShortcut { .. } => "create-shortcut",
            Self::RegisterApplication { .. } => "register-application",
            Self::InstallService { .. } => "install-service",
        }
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateDirectory { path } => write!(f, "mkdir     {path}"),
            Self::InstallFile { target, size, .. } => {
                write!(f, "file      {target} ({size} bytes)")
            }
            Self::AddPathEntry { directory, scope } => write!(f, "path      {directory} ({scope})"),
            Self::CreateShortcut {
                name,
                target,
                location,
                ..
            } => write!(f, "shortcut  {name} -> {target} ({location})"),
            Self::RegisterApplication { display_name, .. } => write!(f, "register  {display_name}"),
            Self::InstallService {
                id, command, scope, ..
            } => {
                write!(f, "service   {id} -> {command} ({scope})")
            }
        }
    }
}

/// sha-256 of a file.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Digest(pub [u8; 32]);

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{self}")
    }
}
