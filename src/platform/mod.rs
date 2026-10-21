//! what the transaction needs from the OS: root locations, PATH,
//! shortcuts and the uninstall registration. file copies are plain `std::fs`
//! and stay in `transaction`.

/// the only place kago calls into the OS by FFI.
mod json;
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

use std::path::{Path, PathBuf};

use crate::install::{Roots, Scope, Slot};
use crate::manifest::Value;
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

    /// the value at `slot`; `None` when there is none.
    ///
    /// # Errors
    ///
    /// when it cannot be read, or this OS has no such place.
    fn value(&self, slot: &Slot) -> Result<Option<Value>, String> {
        match slot {
            Slot::Json { file, key } => Ok(json::value(file, key)?.map(Value)),
            _ => Err(format!("{slot}: not on this OS")),
        }
    }

    /// sets the value at `slot`, or removes it for `None`.
    ///
    /// # Errors
    ///
    /// when it cannot be written, or this OS has no such place.
    fn set_value(&self, slot: &Slot, value: Option<&Value>) -> Result<(), String> {
        match slot {
            Slot::Json { file, key } => json::set(file, key, value.map(|v| &v.0)),
            _ => Err(format!("{slot}: not on this OS")),
        }
    }

    /// adds `item` to the list at `slot`. `false` when it was there
    /// already, so the install does not own it.
    ///
    /// # Errors
    ///
    /// as [`Platform::set_value`], or when the value there is no list.
    fn add_item(&self, slot: &Slot, item: &Value) -> Result<bool, String> {
        match slot {
            Slot::Json { file, key } => json::add(file, key, &item.0),
            _ => Err(format!("{slot}: not a list on this OS")),
        }
    }

    /// takes `item` out of the list at `slot`. missing is fine.
    ///
    /// # Errors
    ///
    /// as [`Platform::add_item`].
    fn remove_item(&self, slot: &Slot, item: &Value) -> Result<(), String> {
        match slot {
            Slot::Json { file, key } => json::remove(file, key, &item.0),
            _ => Err(format!("{slot}: not a list on this OS")),
        }
    }

    /// the inbound firewall rule called `name`, if there is one.
    ///
    /// # Errors
    ///
    /// when the firewall cannot be read here.
    fn firewall_rule(&self, name: &str, group: &str) -> Result<Option<Firewall>, String> {
        let _ = (name, group);
        Err("firewall rules are Windows-only for now".to_owned())
    }

    /// makes the rule `name`, in `group`, say what `rule` says, replacing
    /// one of that name; `None` removes it.
    ///
    /// # Errors
    ///
    /// without administrator rights, or where there is no such firewall.
    fn set_firewall_rule(
        &self,
        name: &str,
        group: &str,
        rule: Option<&Firewall>,
    ) -> Result<(), String> {
        let _ = (name, group, rule);
        Err("firewall rules are Windows-only for now".to_owned())
    }

    /// the Windows service called `name` as it is set up now.
    ///
    /// # Errors
    ///
    /// when services cannot be read here.
    fn service(&self, name: &str) -> Result<Option<WindowsService>, String> {
        let _ = name;
        Err("services for everyone are Windows-only for now".to_owned())
    }

    /// creates or changes the service `name` and starts it when it starts
    /// automatically; `None` stops and deletes it.
    ///
    /// # Errors
    ///
    /// without administrator rights, or when it does not start.
    fn set_service(&self, name: &str, service: Option<&WindowsService>) -> Result<(), String> {
        let _ = (name, service);
        Err("services for everyone are Windows-only for now".to_owned())
    }

    /// stops the service `name` if it runs; keeps its setup.
    ///
    /// # Errors
    ///
    /// without administrator rights, or when it does not stop.
    fn stop_service(&self, name: &str) -> Result<(), String> {
        let _ = name;
        Ok(())
    }

    /// whether `dir` is a folder the system or other programs use too, or
    /// holds one, like the user's Documents: removing an app's data never
    /// takes it whole.
    fn shared_folder(&self, dir: &Path) -> bool {
        let _ = dir;
        false
    }

    /// has the file or empty folder at `path` deleted when the computer
    /// next starts, for one still in use now.
    ///
    /// # Errors
    ///
    /// when this OS cannot, or without the rights to ask.
    fn delete_at_restart(&self, path: &Path) -> Result<(), String> {
        Err(format!(
            "{} cannot be removed at restart here",
            path.display()
        ))
    }

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

    /// whether `check` finds what it looks for on this machine.
    fn satisfied(&self, check: &crate::manifest::Check) -> bool {
        satisfied(check)
    }

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

/// the value at `key` in the JSON file `file`, for platforms that keep
/// other places themselves.
///
/// # Errors
///
/// when the file cannot be read or is not a JSON object.
pub fn json_value(file: &Path, key: &[String]) -> Result<Option<Value>, String> {
    Ok(json::value(file, key)?.map(Value))
}

/// sets `key` in the JSON file `file`, or removes it for `None`.
///
/// # Errors
///
/// when the file cannot be read or written.
pub fn set_json_value(file: &Path, key: &[String], value: Option<&Value>) -> Result<(), String> {
    json::set(file, key, value.map(|v| &v.0))
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

/// whether `check` finds what it looks for on this machine.
#[must_use]
pub fn satisfied(check: &crate::manifest::Check) -> bool {
    use crate::manifest::Check;
    match check {
        Check::File { path } => Path::new(&expand_env(path)).exists(),
        Check::Json { file, key } => json::value(Path::new(&expand_env(file)), key)
            .ok()
            .flatten()
            .is_some_and(|v| match v {
                serde_json::Value::Null | serde_json::Value::Bool(false) => false,
                serde_json::Value::Number(n) => n.as_f64() != Some(0.0),
                serde_json::Value::String(s) => !s.is_empty(),
                serde_json::Value::Array(a) => !a.is_empty(),
                serde_json::Value::Object(o) => !o.is_empty(),
                serde_json::Value::Bool(true) => true,
            }),
        Check::Registry { key, value } => {
            #[cfg(windows)]
            {
                windows::registry_has(key, value.as_deref())
            }
            #[cfg(not(windows))]
            {
                let _ = (key, value);
                false
            }
        }
    }
}

/// `%NAME%` replaced by the environment variable `NAME`, like Windows
/// does; unknown names stay as written.
fn expand_env(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        let Some(len) = rest[start + 1..].find('%') else {
            break;
        };
        let name = &rest[start + 1..start + 1 + len];
        out.push_str(&rest[..start]);
        match std::env::var(name) {
            Ok(value) if !name.is_empty() => out.push_str(&value),
            _ => out.push_str(&rest[start..=start + 1 + len]),
        }
        rest = &rest[start + 2 + len..];
    }
    out.push_str(rest);
    out
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

/// `exe` and `arguments` as one Windows command line, each quoted the way
/// programs read them back.
#[must_use]
pub fn command_line(exe: &Path, arguments: &[String]) -> String {
    #[cfg(windows)]
    {
        let mut command = windows::quote(&exe.display().to_string());
        let rest = windows::join_arguments(arguments);
        if !rest.is_empty() {
            command.push(' ');
            command.push_str(&rest);
        }
        command
    }
    #[cfg(not(windows))]
    {
        std::iter::once(exe.display().to_string())
            .chain(arguments.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// a Windows service: runs for everyone, as the `LocalSystem` account, without anyone
/// signed in.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WindowsService {
    pub display_name: String,
    /// the program, quoted, and its arguments.
    pub command_line: String,
    /// starts with Windows; otherwise only when asked.
    pub automatic: bool,
    pub restart_on_failure: bool,
}

/// a firewall rule letting a program accept connections: on private and
/// work networks, and on public ones when `public`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Firewall {
    pub program: PathBuf,
    pub public: bool,
    /// only this protocol, on `ports` when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<crate::manifest::Protocol>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ports: String,
}

/// held while this process is the only setup or uninstaller working on
/// a package; let go on drop.
pub struct Instance {
    #[cfg(windows)]
    _held: windows::instance::Instance,
}

/// claims `package` for this process, waiting up to `wait_ms` for another
/// setup to finish. `None` while another one runs.
#[must_use]
pub fn claim(package: &str, wait_ms: u32) -> Option<Instance> {
    #[cfg(windows)]
    {
        windows::instance::claim(&format!("kago-{package}"), wait_ms)
            .map(|held| Instance { _held: held })
    }
    #[cfg(not(windows))]
    {
        let _ = (package, wait_ms);
        Some(Instance {})
    }
}

/// shows `text` in a message box, for when there is no window yet.
pub fn tell(title: &str, text: &str) {
    #[cfg(windows)]
    {
        windows::instance::tell(title, text);
    }
    #[cfg(not(windows))]
    {
        eprintln!("{title}: {text}");
    }
}

/// bytes free for this user on the drive `path` is on, or would be on
/// once created. `None` when it cannot be told.
#[must_use]
pub fn free_space(path: &Path) -> Option<u64> {
    // the nearest folder that exists: the install folder usually does not.
    let existing = path.ancestors().find(|p| p.is_dir())?;
    #[cfg(windows)]
    {
        windows::folder::free_space(existing)
    }
    #[cfg(not(windows))]
    {
        let _ = existing;
        None
    }
}

/// names of the programs that have any of `files` open, like the app an
/// update would replace. empty where this OS cannot tell.
///
/// # Errors
///
/// when the system cannot be asked.
pub fn programs_using(files: &[PathBuf]) -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        windows::restart::programs_using(files)
    }
    #[cfg(not(windows))]
    {
        let _ = files;
        Ok(Vec::new())
    }
}

/// closes the programs that have any of `files` open: asks first, then
/// ends those that stay.
///
/// # Errors
///
/// when one cannot be closed.
pub fn close_programs(files: &[PathBuf]) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::restart::close_programs(files)
    }
    #[cfg(not(windows))]
    {
        let _ = files;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Check;

    #[test]
    fn environment_names_expand_and_unknown_ones_stay() {
        let path = std::env::var("PATH").unwrap();
        assert_eq!(expand_env("%PATH%;x"), format!("{path};x"));
        assert_eq!(expand_env(r"%KAGO_NO_SUCH%\a"), r"%KAGO_NO_SUCH%\a");
        assert_eq!(expand_env("50%"), "50%");
    }

    #[cfg(windows)]
    #[test]
    fn json_checks_want_something_there() {
        let dir = std::env::temp_dir().join(format!("kago-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("paths.json");
        std::fs::write(
            &file,
            r#"{"runtime":["C:\\SteamVR"],"empty":[],"off":false}"#,
        )
        .unwrap();
        let check = |key: &str| {
            satisfied(&crate::manifest::Check::Json {
                file: file.display().to_string(),
                key: vec![key.to_owned()],
            })
        };
        assert!(check("runtime"));
        assert!(!check("empty"));
        assert!(!check("off"));
        assert!(!check("missing"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_service_reads_as_none() {
        assert_eq!(
            windows::service::service("kago-test-no-such-service"),
            Ok(None)
        );
        let spooler = windows::service::service("Spooler").unwrap();
        assert!(spooler.is_some_and(|s| s.command_line.to_ascii_lowercase().contains("spoolsv")));
    }

    #[cfg(windows)]
    #[test]
    fn a_missing_firewall_rule_reads_as_none() {
        assert_eq!(
            windows::firewall::rule("kago test: no such rule", "kago test"),
            Ok(None)
        );
    }

    #[cfg(windows)]
    #[test]
    fn registry_checks_read_the_machine() {
        let windows = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        let has = |key: &str, value: Option<&str>| {
            satisfied(&Check::Registry {
                key: key.to_owned(),
                value: value.map(str::to_owned),
            })
        };
        assert!(has(windows, None));
        assert!(has(windows, Some("ProductName")));
        assert!(!has(windows, Some("KagoNoSuchValue")));
        assert!(!has(r"HKLM\SOFTWARE\KagoNoSuchKey", None));
        assert!(!has(r"HKXX\SOFTWARE", None));
    }
}
