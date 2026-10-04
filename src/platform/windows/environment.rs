//! PATH as a parsed list in the registry, plus the `WM_SETTINGCHANGE`
//! broadcast so new shells see it.

use std::path::Path;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
};
use windows::core::HSTRING;

use super::registry::{self, Key};
use crate::install::Scope;

const USER_ENV: &str = "Environment";
const MACHINE_ENV: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

fn env_key(scope: Scope) -> &'static str {
    match scope {
        Scope::User => USER_ENV,
        Scope::Machine => MACHINE_ENV,
    }
}

/// same directory regardless of case and trailing `\`.
fn same(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_lowercase();
    norm(a) == norm(b)
}

/// `path` with `entry` appended, or `None` if it is already listed.
pub fn with_entry(path: &str, entry: &str) -> Option<String> {
    if path.split(';').any(|e| same(e, entry)) {
        return None;
    }
    let trimmed = path.trim_end_matches(';');
    Some(if trimmed.is_empty() {
        entry.to_owned()
    } else {
        format!("{trimmed};{entry}")
    })
}

/// `path` without `entry`. every other entry stays, in order, as written.
pub fn without_entry(path: &str, entry: &str) -> String {
    path.split(';')
        .filter(|e| !e.is_empty() && !same(e, entry))
        .collect::<Vec<_>>()
        .join(";")
}

pub fn add(scope: Scope, entry: &Path) -> Result<bool, String> {
    let entry = entry.display().to_string();
    let key = Key::create(registry::hive(scope), env_key(scope))?;
    let current = key.text("Path")?;
    let (value, expand) = current.map_or((String::new(), true), |t| (t.value, t.expand));
    let Some(next) = with_entry(&value, &entry) else {
        return Ok(false);
    };
    key.set_text("Path", &next, expand)?;
    broadcast();
    Ok(true)
}

pub fn remove(scope: Scope, entry: &Path) -> Result<(), String> {
    let entry = entry.display().to_string();
    let Some(key) = Key::open(registry::hive(scope), env_key(scope))? else {
        return Ok(());
    };
    let Some(current) = key.text("Path")? else {
        return Ok(());
    };
    let next = without_entry(&current.value, &entry);
    if next != current.value {
        key.set_text("Path", &next, current.expand)?;
        broadcast();
    }
    Ok(())
}

/// tells Explorer and other top-level windows the environment changed.
/// windows that hang are skipped after a timeout.
fn broadcast() {
    let name = HSTRING::from("Environment");
    // SAFETY: lparam points at a nul-terminated wide string that outlives the call.
    unsafe {
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(name.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            5000,
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_once() {
        assert_eq!(with_entry("", r"C:\a"), Some(r"C:\a".into()));
        assert_eq!(with_entry(r"C:\x;", r"C:\a"), Some(r"C:\x;C:\a".into()));
        assert_eq!(with_entry(r"C:\x;c:\A\", r"C:\a"), None);
    }

    #[test]
    fn removes_only_that_entry() {
        assert_eq!(
            without_entry(r"C:\x;C:\a;%USERPROFILE%\bin;C:\A\", r"C:\a"),
            r"C:\x;%USERPROFILE%\bin"
        );
        assert_eq!(without_entry(r"C:\x;C:\ab", r"C:\a"), r"C:\x;C:\ab");
    }
}
