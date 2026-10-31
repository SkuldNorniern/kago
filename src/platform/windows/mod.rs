//! Windows: per-user installs under `%LOCALAPPDATA%\Programs`, PATH in
//! `HKCU\Environment`, Start Menu `.lnk`, uninstall entry in `HKCU`.

mod admin_only;
pub mod elevation;
mod environment;
pub mod firewall;
pub mod folder;
pub mod instance;
mod registry;
pub mod resource;

pub use shortcut::{join_arguments, quote};
pub mod restart;
pub mod service;
mod shared;
mod shortcut;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{Platform, Registration};
use crate::install::{Roots, Scope, Slot, folder_name};
use crate::manifest::{Root, Value};
use registry::Key;

const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
/// programs started when the user signs in.
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub struct Windows;

const fn scope_of(machine: bool) -> Scope {
    if machine { Scope::Machine } else { Scope::User }
}

fn env_dir(name: &str) -> Result<PathBuf, String> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| format!("%{name}% is not set"))
}

/// the desktop the person sees, which `OneDrive` may have moved out of the profile.
fn desktop() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    // SAFETY: the folder id is a static GUID and no token is given; the path
    // is read before CoTaskMemFree frees the buffer SHGetKnownFolderPath made.
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None).ok()?;
        let text = path.to_string();
        CoTaskMemFree(Some(path.0.cast()));
        text.ok().map(PathBuf::from)
    }
}

/// whether what `detect` names is on this machine.
/// whether the registry key `key` (`HKLM\...`, `HKCU\...`) exists and,
/// when `value` is given, holds that value set to something other than 0
/// or empty.
pub fn registry_has(key: &str, value: Option<&str>) -> bool {
    use ::windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let Some((hive, path)) = key.split_once('\\') else {
        return false;
    };
    let hive = match hive.to_ascii_uppercase().as_str() {
        "HKLM" | "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        "HKCU" | "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        _ => return false,
    };
    let Ok(Some(key)) = Key::open_read(hive, path) else {
        return false;
    };
    let Some(value) = value else {
        return true;
    };
    match key.dword(value) {
        Ok(Some(n)) => n != 0,
        _ => key
            .text(value)
            .ok()
            .flatten()
            .is_some_and(|t| !t.value.is_empty()),
    }
}

impl Platform for Windows {
    fn roots(&self, scope: Scope, name: &str) -> Result<Roots, String> {
        let folder = folder_name(name);
        if scope == Scope::Machine {
            // for everyone, like NSIS with `SetShellVarContext all`.
            let programs = env_dir("ProgramFiles")?;
            let shared = env_dir("ProgramData")?;
            let public = env_dir("PUBLIC")?;
            let app = programs.join(&folder);
            let data = shared.join(&folder);
            return Ok(Roots(BTreeMap::from([
                (Root::App, app.clone()),
                (Root::Bin, app.join("bin")),
                (Root::Config, data.clone()),
                (Root::Data, data.clone()),
                (Root::Cache, data.join("Cache")),
                (Root::Logs, data.join("Logs")),
                (Root::State, data.join("State")),
                (Root::Desktop, public.join("Desktop")),
                (
                    Root::Applications,
                    shared.join(r"Microsoft\Windows\Start Menu\Programs"),
                ),
            ])));
        }
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
            (
                Root::Desktop,
                desktop().unwrap_or_else(|| home.join("Desktop")),
            ),
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
            Scope::Machine => Ok(env_dir("ProgramData")?.join("Kago")),
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

    fn value(&self, slot: &Slot) -> Result<Option<Value>, String> {
        match slot {
            Slot::Json { file, key } => Ok(super::json::value(file, key)?.map(Value)),
            Slot::Registry { machine, key, name } => {
                Ok(registry::value(*machine, key, name)?.map(Value))
            }
            Slot::Environment { machine, name } => {
                Ok(environment::value(scope_of(*machine), name)?.map(Value::text))
            }
        }
    }

    fn set_value(&self, slot: &Slot, value: Option<&Value>) -> Result<(), String> {
        match slot {
            Slot::Json { file, key } => super::json::set(file, key, value.map(|v| &v.0)),
            Slot::Registry { machine, key, name } => {
                registry::set_value(*machine, key, name, value.map(|v| &v.0))?;
                // a file type or URL protocol: Explorer shows the change
                // without a restart.
                if key.to_ascii_lowercase().starts_with(r"software\classes") {
                    shortcut::associations_changed();
                }
                Ok(())
            }
            Slot::Environment { machine, name } => {
                let text = value.map(ToString::to_string);
                environment::set(scope_of(*machine), name, text.as_deref())
            }
        }
    }

    fn add_item(&self, slot: &Slot, item: &Value) -> Result<bool, String> {
        match slot {
            Slot::Json { file, key } => super::json::add(file, key, &item.0),
            Slot::Environment { machine, name } => {
                environment::add_item(scope_of(*machine), name, &item.to_string())
            }
            Slot::Registry { .. } => Err(format!("{slot}: a registry value is not a list")),
        }
    }

    fn remove_item(&self, slot: &Slot, item: &Value) -> Result<(), String> {
        match slot {
            Slot::Json { file, key } => super::json::remove(file, key, &item.0),
            Slot::Environment { machine, name } => {
                environment::remove_item(scope_of(*machine), name, &item.to_string())
            }
            Slot::Registry { .. } => Err(format!("{slot}: a registry value is not a list")),
        }
    }

    fn firewall_rule(&self, name: &str, group: &str) -> Result<Option<super::Firewall>, String> {
        firewall::rule(name, group)
    }

    fn set_firewall_rule(
        &self,
        name: &str,
        group: &str,
        rule: Option<&super::Firewall>,
    ) -> Result<(), String> {
        firewall::set_rule(name, group, rule)
    }

    fn service(&self, name: &str) -> Result<Option<super::WindowsService>, String> {
        service::service(name)
    }

    fn set_service(
        &self,
        name: &str,
        wanted: Option<&super::WindowsService>,
    ) -> Result<(), String> {
        service::set_service(name, wanted)
    }

    fn stop_service(&self, name: &str) -> Result<(), String> {
        service::stop_service(name)
    }

    fn secure_state(&self, dir: &Path) -> Result<(), String> {
        if !elevation::is_elevated() {
            return Ok(());
        }
        admin_only::admin_only(dir)
    }

    fn shared_folder(&self, dir: &Path) -> bool {
        shared::shared(dir)
    }

    fn delete_at_restart(&self, path: &Path) -> Result<(), String> {
        use windows::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
        // SAFETY: the path string lives until the call returns; no target
        // means delete.
        unsafe {
            MoveFileExW(
                &windows::core::HSTRING::from(path),
                windows::core::PCWSTR::null(),
                MOVEFILE_DELAY_UNTIL_REBOOT,
            )
        }
        .map_err(|e| format!("{}: {}", path.display(), e.message()))
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

    fn create_shortcut(
        &self,
        link: &Path,
        target: &Path,
        arguments: &[String],
        icon: Option<&Path>,
    ) -> Result<(), String> {
        shortcut::create(link, target, arguments, icon)
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
        let asking = r
            .uninstaller
            .as_ref()
            .map_or_else(|| r.uninstall.clone(), |u| format!("\"{}\"", u.display()));
        key.set_text("UninstallString", &asking, false)?;
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

    /// reads only: the desktop of the user running the tests is a folder.
    #[test]
    fn desktop_is_a_folder_that_exists() {
        let dir = desktop().expect("a desktop folder");
        assert!(dir.is_dir(), "{}", dir.display());
    }
}
