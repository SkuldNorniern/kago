//! what the transaction needs from the OS: root locations, PATH,
//! shortcuts and the uninstall registration. file copies are plain `std::fs`
//! and stay in `transaction`.

/// the only place kago calls into the OS by FFI.
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
    /// full command line that uninstalls this package.
    pub uninstall: String,
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

    /// # Errors
    ///
    /// when the shortcut cannot be written.
    fn create_shortcut(
        &self,
        link: &Path,
        target: &Path,
        arguments: &[String],
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
