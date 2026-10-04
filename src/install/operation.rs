use std::fmt;
use std::path::PathBuf;

use super::Scope;
use crate::manifest::{HookPhase, Ownership, SemanticPath, ShortcutLocation};

/// one step of an install. platforms carry these out; nothing here
/// touches the machine.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
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
    /// registers a SteamVR driver folder with SteamVR.
    RegisterSteamVrDriver {
        directory: SemanticPath,
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
    /// runs the package's own `command`: now for `post-install`, at
    /// uninstall for `pre-uninstall`.
    RunHook {
        phase: HookPhase,
        command: SemanticPath,
        arguments: Vec<String>,
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
            Self::RegisterSteamVrDriver { .. } => "register-steamvr-driver",
            Self::CreateShortcut { .. } => "create-shortcut",
            Self::RegisterApplication { .. } => "register-application",
            Self::RunHook { .. } => "run-hook",
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
            Self::RegisterSteamVrDriver { directory } => write!(f, "steamvr   {directory}"),
            Self::CreateShortcut {
                name,
                target,
                location,
                ..
            } => write!(f, "shortcut  {name} -> {target} ({location})"),
            Self::RegisterApplication { display_name, .. } => write!(f, "register  {display_name}"),
            Self::RunHook { phase, command, .. } => write!(f, "hook      {phase} {command}"),
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

impl std::str::FromStr for Digest {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || format!("`{s}` is not a sha-256 hex digest");
        if s.len() != 64 || !s.is_ascii() {
            return Err(bad());
        }
        let mut out = [0; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| bad())?;
        }
        Ok(Self(out))
    }
}

crate::string_serde!(Digest);
