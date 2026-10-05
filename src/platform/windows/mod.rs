//! Windows: per-user installs under `%LOCALAPPDATA%\Programs`, PATH in
//! `HKCU\Environment`, Start Menu `.lnk`, uninstall entry in `HKCU`.

mod environment;
pub mod folder;
mod registry;
pub mod resource;
mod shortcut;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{Platform, Registration};
use crate::install::{Roots, Scope, folder_name};
use crate::manifest::{Detect, Root};
use registry::Key;

const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
/// programs started when the user signs in.
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub struct Windows;

fn env_dir(name: &str) -> Result<PathBuf, String> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| format!("%{name}% is not set"))
}

/// whether what `detect` names is on this machine.
pub fn detect(detect: Detect) -> bool {
    match detect {
        Detect::SteamVr => openvr_paths().is_ok_and(|f| super::openvr::runtime_installed(&f)),
    }
}

/// SteamVR's per-user path file, read by every OpenVR runtime and app.
fn openvr_paths() -> Result<PathBuf, String> {
    Ok(env_dir("LOCALAPPDATA")?.join(r"openvr\openvrpaths.vrpath"))
}

fn machine_unsupported() -> String {
    "machine scope needs the elevation helper, which is not done yet; use `--scope user`".to_owned()
}

impl Platform for Windows {
    fn roots(&self, scope: Scope, name: &str) -> Result<Roots, String> {
        if scope == Scope::Machine {
            return Err(machine_unsupported());
        }
        let folder = folder_name(name);
        let local = env_dir("LOCALAPPDATA")?;
        let roaming = env_dir("APPDATA")?;
        let home = env_dir("USERPROFILE")?;
        let app = local.join("Programs").join(&folder);
        let data = local.join(&folder);
        Ok(Roots(BTreeMap::from([
            (Root::App, app.clone()),
            (Root::Bin, app.join("bin")),
            (Root::Config, roaming.join(&folder)),
            (Root::Data, data.clone()),
            (Root::Cache, data.join("Cache")),
            (Root::Logs, data.join("Logs")),
            (Root::State, data.join("State")),
            (Root::Desktop, home.join("Desktop")),
            (
                Root::Applications,
                roaming.join(r"Microsoft\Windows\Start Menu\Programs"),
            ),
            (Root::UserHome, home),
        ])))
    }

    fn state_dir(&self, scope: Scope) -> Result<PathBuf, String> {
        match scope {
            Scope::User => Ok(env_dir("LOCALAPPDATA")?.join("Kago")),
            Scope::Machine => Err(machine_unsupported()),
        }
    }

    fn shortcut_file(&self, name: &str) -> String {
        format!("{}.lnk", folder_name(name))
    }

    fn add_path_entry(&self, scope: Scope, entry: &Path) -> Result<bool, String> {
        environment::add(scope, entry)
    }

    fn remove_path_entry(&self, scope: Scope, entry: &Path) -> Result<(), String> {
        environment::remove(scope, entry)
    }

    fn add_steamvr_driver(&self, directory: &Path) -> Result<bool, String> {
        super::openvr::add(&openvr_paths()?, directory)
    }

    fn remove_steamvr_driver(&self, directory: &Path) -> Result<(), String> {
        super::openvr::remove(&openvr_paths()?, directory)
    }

    fn login_item(&self, name: &str) -> Result<Option<String>, String> {
        let Some(key) = Key::open_read(registry::hive(Scope::User), RUN)? else {
            return Ok(None);
        };
        Ok(key.text(name)?.map(|t| t.value))
    }

    fn add_login_item(
        &self,
        name: &str,
        exe: &Path,
        arguments: &[String],
    ) -> Result<String, String> {
        let mut command = shortcut::quote(&exe.display().to_string());
        let rest = shortcut::join_arguments(arguments);
        if !rest.is_empty() {
            command.push(' ');
            command.push_str(&rest);
        }
        Key::create(registry::hive(Scope::User), RUN)?.set_text(name, &command, false)?;
        Ok(command)
    }

    fn set_login_item(&self, name: &str, command: Option<&str>) -> Result<(), String> {
        let key = Key::create(registry::hive(Scope::User), RUN)?;
        match command {
            Some(command) => key.set_text(name, command, false),
            None => key.delete_value(name),
        }
    }

    fn os_version(&self) -> Option<String> {
        let key = Key::open_read(
            windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        )
        .ok()??;
        let major = key.dword("CurrentMajorVersionNumber").ok()??;
        let minor = key.dword("CurrentMinorVersionNumber").ok()??;
        let build = key.text("CurrentBuildNumber").ok()??.value;
        Some(format!("{major}.{minor}.{build}"))
    }

    fn has(&self, what: Detect) -> bool {
        detect(what)
    }

    fn create_shortcut(
        &self,
        link: &Path,
        target: &Path,
        arguments: &[String],
    ) -> Result<(), String> {
        shortcut::create(link, target, arguments)
    }

    fn register(&self, scope: Scope, r: &Registration) -> Result<String, String> {
        let path = format!(r"{UNINSTALL}\{}", r.package);
        let key = Key::create(registry::hive(scope), &path)?;
        key.set_text("DisplayName", &r.display_name, false)?;
        key.set_text("DisplayVersion", &r.version.to_string(), false)?;
        if let Some(publisher) = &r.publisher {
            key.set_text("Publisher", publisher, false)?;
        }
        key.set_text(
            "InstallLocation",
            &r.install_location.display().to_string(),
            false,
        )?;
        if let Some(icon) = &r.icon {
            key.set_text("DisplayIcon", &icon.display().to_string(), false)?;
        }
        key.set_text("UninstallString", &r.uninstall, false)?;
        key.set_text("QuietUninstallString", &r.uninstall, false)?;
        key.set_dword("NoModify", 1)?;
        key.set_dword("NoRepair", 1)?;
        key.set_dword(
            "EstimatedSize",
            u32::try_from(r.size.div_ceil(1024)).unwrap_or(u32::MAX),
        )?;
        Ok(path)
    }

    fn unregister(&self, scope: Scope, key: &str) -> Result<(), String> {
        if !key.starts_with(UNINSTALL) {
            return Err(format!(
                "refusing to delete registry key outside {UNINSTALL}: {key}"
            ));
        }
        registry::delete_tree(registry::hive(scope), key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// reads only: the version of the machine running the tests.
    #[test]
    fn os_version_reads_this_machine() {
        let version = Windows.os_version().expect("a windows version");
        assert!(version.split('.').count() == 3, "{version}");
        assert!(crate::install::at_least(&version, "10.0"), "{version}");
    }
}
