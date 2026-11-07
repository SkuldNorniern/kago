use std::fmt;
use std::path::PathBuf;

use super::Scope;
use crate::manifest::{Edit, Ownership, Place, RunAt, SemanticPath, ShortcutLocation, Value};

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
    CreateShortcut {
        name: String,
        target: SemanticPath,
        location: ShortcutLocation,
        arguments: Vec<String>,
        /// an installed `.ico` to show; the target's own icon otherwise.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<SemanticPath>,
    },
    /// uninstall entry, start menu / desktop file metadata.
    RegisterApplication {
        display_name: String,
        entrypoint: Option<SemanticPath>,
        /// an installed `.ico` for the uninstall entry.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon: Option<SemanticPath>,
    },
    /// starts a program: now for `install`, first thing at uninstall for
    /// `uninstall`. `undo` runs on rollback and at uninstall.
    Run {
        phase: RunAt,
        command: Invocation,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        undo: Option<Invocation>,
        /// at uninstall: stops it when the program fails.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        required: bool,
    },
    /// per user, a program started at sign-in; for everyone, a Windows
    /// service.
    InstallService {
        id: String,
        command: SemanticPath,
        arguments: Vec<String>,
        scope: Scope,
        /// what Windows shows for it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// a service started only when asked, not with Windows.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        manual: bool,
        /// a service started again after it crashes.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        restart: bool,
    },
    /// sets a value outside the package: in a JSON file, the registry or
    /// the environment. uninstall puts it back.
    Change {
        place: Place,
        edit: Edit,
        value: Value,
    },
    /// lets `program` accept connections through Windows Firewall. needs
    /// an install for everyone; a per-user one leaves it to Windows to
    /// ask the first time the program listens.
    AllowThroughFirewall {
        name: String,
        program: SemanticPath,
        public: bool,
        /// only this protocol, on `ports` when given.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        protocol: Option<crate::manifest::Protocol>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        ports: String,
    },
    /// puts the driver package whose `.inf` the install placed at `inf`
    /// into the driver store, makes a device for each hardware id in
    /// `devices` and installs the driver on it. needs an install for
    /// everyone.
    InstallDriver {
        inf: SemanticPath,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        devices: Vec<String>,
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
            Self::Run { .. } => "run",
            Self::InstallService { .. } => "install-service",
            Self::AllowThroughFirewall { .. } => "allow-through-firewall",
            Self::InstallDriver { .. } => "install-driver",
            Self::Change { .. } => "change",
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
            Self::Run { phase, command, .. } => write!(f, "run       {command} (at {phase})"),
            Self::InstallService {
                id, command, scope, ..
            } => {
                write!(f, "service   {id} -> {command} ({scope})")
            }
            Self::Change { place, edit, value } => {
                write!(f, "{:<9} {place} {edit} {value}", place.kind())
            }
            Self::AllowThroughFirewall {
                name,
                program,
                public,
                protocol,
                ports,
            } => {
                let networks = if *public {
                    "all networks"
                } else {
                    "private networks"
                };
                let only = match (protocol, ports.is_empty()) {
                    (Some(p), true) => format!(", {p} only"),
                    (Some(p), false) => format!(", {p} {ports}"),
                    (None, _) => String::new(),
                };
                write!(f, "firewall  {name} -> {program} ({networks}{only})")
            }
            Self::InstallDriver { inf, devices } => {
                write!(f, "driver    {inf}")?;
                if !devices.is_empty() {
                    write!(f, " (devices {})", devices.join(", "))?;
                }
                Ok(())
            }
        }
    }
}

/// what `run` starts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "from", rename_all = "kebab-case")]
pub enum Program {
    /// a file the package installs.
    Installed { path: SemanticPath },
    /// a program on the machine, found on PATH. only installs for one
    /// person may start one.
    System { name: String },
}

/// a program with its arguments; `{app}` and the other roots in them
/// become real paths at install.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Invocation {
    pub program: Program,
    pub arguments: Vec<String>,
}

impl fmt::Display for Invocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.program {
            Program::Installed { path } => write!(f, "{path}")?,
            Program::System { name } => f.write_str(name)?,
        }
        for a in &self.arguments {
            write!(f, " {a}")?;
        }
        Ok(())
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
