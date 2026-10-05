//! what the transaction needs from the OS: root locations, PATH,
//! shortcuts and the uninstall registration. file copies are plain `std::fs`
//! and stay in `transaction`.

/// the only place kago calls into the OS by FFI.
mod openvr;
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

use std::path::{Path, PathBuf};

use crate::install::{Roots, Scope};
use crate::package::{PackageId, Version};

/// uninstall entry or desktop registration for one install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub package: PackageId,
    pub display_name: String,
    pub version: Version,
    pub publisher: Option<String>,
    pub install_location: PathBuf,
    pub icon: Option<PathBuf>,
    /// full command line that uninstalls this package without asking.
    pub uninstall: String,
    /// the package's own uninstaller, which asks first; what Settings runs.
    pub uninstaller: Option<PathBuf>,
    pub size: u64,
}

pub trait Platform {
    /// locations of the semantic roots for `name` installed in `scope`.
    ///
    /// # Errors
    ///
    /// when the scope is not supported here yet.
    fn roots(&self, scope: Scope, name: &str) -> Result<Roots, String>;

    /// kago's own state for `scope`: receipts, journals, locks.
    ///
    /// # Errors
    ///
    /// when the location cannot be found.
    fn state_dir(&self, scope: Scope) -> Result<PathBuf, String>;

    /// shortcut file name for a display name, like `Ozone.lnk`.
    fn shortcut_file(&self, name: &str) -> String;

    /// adds `entry` to PATH. `false` when it was already there, so the
    /// install does not own it.
    ///
    /// # Errors
    ///
    /// when PATH cannot be read or written.
    fn add_path_entry(&self, scope: Scope, entry: &Path) -> Result<bool, String>;

    /// removes `entry` from PATH if present. other entries stay as they were.
    ///
    /// # Errors
    ///
    /// when PATH cannot be read or written.
    fn remove_path_entry(&self, scope: Scope, entry: &Path) -> Result<(), String>;

    /// registers a SteamVR driver folder, as `vrpathreg adddriver` does.
    /// `false` when it was already registered, so the install does not own
    /// it.
    ///
    /// # Errors
    ///
    /// when SteamVR's path file cannot be read or written.
    fn add_steamvr_driver(&self, directory: &Path) -> Result<bool, String>;

    /// removes a SteamVR driver folder from SteamVR. missing is fine.
    ///
    /// # Errors
    ///
    /// when SteamVR's path file cannot be read or written.
    fn remove_steamvr_driver(&self, directory: &Path) -> Result<(), String>;

    /// the command a login item named `name` runs now, if there is one.
    ///
    /// # Errors
    ///
    /// when the login items cannot be read.
    fn login_item(&self, name: &str) -> Result<Option<String>, String>;

    /// starts `exe` with `arguments` when the user signs in, under `name`.
    /// returns the command line it wrote.
    ///
    /// # Errors
    ///
    /// when the login items cannot be written.
    fn add_login_item(
        &self,
        name: &str,
        exe: &Path,
        arguments: &[String],
    ) -> Result<String, String>;

    /// puts back a login item exactly as `login_item` read it, or removes
    /// it for `None`.
    ///
    /// # Errors
    ///
    /// when the login items cannot be written.
    fn set_login_item(&self, name: &str, command: Option<&str>) -> Result<(), String>;

    /// this OS's version as dotted numbers, like `10.0.22631`.
    fn os_version(&self) -> Option<String>;

    /// whether what `detect` names is on this machine.
    fn has(&self, detect: crate::manifest::Detect) -> bool;

    /// # Errors
    ///
    /// when the shortcut cannot be written.
    fn create_shortcut(
        &self,
        link: &Path,
        target: &Path,
        arguments: &[String],
        icon: Option<&Path>,
    ) -> Result<(), String>;

    /// registers the install, returns the key to unregister it with.
    ///
    /// # Errors
    ///
    /// when the registration cannot be written.
    fn register(&self, scope: Scope, registration: &Registration) -> Result<String, String>;

    /// removes a registration. missing is fine.
    ///
    /// # Errors
    ///
    /// when it exists but cannot be removed.
    fn unregister(&self, scope: Scope, key: &str) -> Result<(), String>;
}

/// the platform kago is running on.
///
/// # Errors
///
/// on an OS kago cannot install on yet.
pub fn host() -> Result<Box<dyn Platform>, String> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows::Windows))
    }
    #[cfg(not(windows))]
    {
        Err("installing on this OS is not supported yet".to_owned())
    }
}

/// whether what a component's `detect` names is on this machine. unknown
/// off Windows for now, which counts as present: nothing is unticked on a
/// guess.
#[must_use]
pub fn detect(detect: crate::manifest::Detect) -> bool {
    #[cfg(windows)]
    {
        windows::detect(detect)
    }
    #[cfg(not(windows))]
    {
        let _ = detect;
        true
    }
}

/// whether this process has administrator rights, which installs for
/// everyone need. elsewhere: always, as there is no UAC to ask.
#[must_use]
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        windows::elevation::is_elevated()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// why a program did not start with administrator rights.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevationError {
    /// the person said no to the prompt.
    Declined,
    Failed(String),
}

impl std::fmt::Display for ElevationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Declined => f.write_str("administrator rights were not given"),
            Self::Failed(e) => f.write_str(e),
        }
    }
}

/// starts `exe` with `arguments` after asking for administrator rights.
/// with `wait`, returns its exit code once it ends.
///
/// # Errors
///
/// when the person says no, or it cannot start.
pub fn run_elevated(exe: &Path, arguments: &[String], wait: bool) -> Result<u32, ElevationError> {
    #[cfg(windows)]
    {
        windows::elevation::run_elevated(exe, arguments, wait)
    }
    #[cfg(not(windows))]
    {
        let _ = (exe, arguments, wait);
        Err(ElevationError::Failed(
            "asking for administrator rights is Windows-only for now".to_owned(),
        ))
    }
}

/// the system's folder picker, over the window `owner` when given.
/// `None` when cancelled or when this OS has none yet.
#[must_use]
pub fn pick_folder(owner: Option<isize>, title: &str, start: Option<&Path>) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        windows::folder::pick(owner, title, start)
    }
    #[cfg(not(windows))]
    {
        let _ = (owner, title, start);
        None
    }
}

/// puts `icon` into the executable at `exe` as its main icon.
///
/// # Errors
///
/// when the file cannot be updated, or off Windows.
pub fn set_exe_icon(exe: &Path, icon: &crate::format::ico::IconGroup) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::resource::set_icon(exe, icon)
    }
    #[cfg(not(windows))]
    {
        let _ = (exe, icon);
        Err("setting an executable icon needs Windows".to_owned())
    }
}
