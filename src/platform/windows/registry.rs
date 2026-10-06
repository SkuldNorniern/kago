//! small safe wrappers over the registry calls kago needs.

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_DWORD, REG_EXPAND_SZ,
    REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW,
    RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{HSTRING, PCWSTR};

use crate::install::Scope;

/// `HKCU` for user installs, `HKLM` for machine installs.
pub fn hive(scope: Scope) -> HKEY {
    match scope {
        Scope::User => HKEY_CURRENT_USER,
        Scope::Machine => HKEY_LOCAL_MACHINE,
    }
}

/// an open key, closed on drop.
pub struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: self.0 came from a successful open/create and is closed once.
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// string value and whether it was `REG_EXPAND_SZ`.
pub struct Text {
    pub value: String,
    pub expand: bool,
}

fn check(e: WIN32_ERROR, what: &str, path: &str) -> Result<(), String> {
    e.ok().map_err(|err| format!("{what} {path}: {err}"))
}

impl Key {
    pub fn create(hive: HKEY, path: &str) -> Result<Self, String> {
        let mut key = HKEY::default();
        // SAFETY: path is a valid wide string for the call, key is a valid out pointer.
        let e = unsafe {
            RegCreateKeyExW(
                hive,
                &HSTRING::from(path),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE,
                None,
                &raw mut key,
                None,
            )
        };
        check(e, "cannot create registry key", path)?;
        Ok(Self(key))
    }

    /// `None` when the key does not exist.
    pub fn open(hive: HKEY, path: &str) -> Result<Option<Self>, String> {
        let mut key = HKEY::default();
        // SAFETY: as in create.
        let e = unsafe {
            RegOpenKeyExW(
                hive,
                &HSTRING::from(path),
                None,
                KEY_READ | KEY_WRITE,
                &raw mut key,
            )
        };
        if e == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(e, "cannot open registry key", path)?;
        Ok(Some(Self(key)))
    }

    /// opened for reading only, which works without admin rights on
    /// `HKLM`. `None` when the key does not exist.
    pub fn open_read(hive: HKEY, path: &str) -> Result<Option<Self>, String> {
        let mut key = HKEY::default();
        // SAFETY: as in create.
        let e = unsafe { RegOpenKeyExW(hive, &HSTRING::from(path), None, KEY_READ, &raw mut key) };
        if e == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(e, "cannot open registry key", path)?;
        Ok(Some(Self(key)))
    }

    /// `None` when the value does not exist or is not a `REG_DWORD`.
    pub fn dword(&self, name: &str) -> Result<Option<u32>, String> {
        let name_w = HSTRING::from(name);
        let mut kind = REG_VALUE_TYPE::default();
        let mut size = 0u32;
        // SAFETY: type and size query with null data, out pointers valid.
        // reading data now fails on a string with "more data".
        let e = unsafe {
            RegQueryValueExW(
                self.0,
                &name_w,
                None,
                Some(&raw mut kind),
                None,
                Some(&raw mut size),
            )
        };
        if e == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(e, "cannot read registry value", name)?;
        if kind != REG_DWORD || size != 4 {
            return Ok(None);
        }
        let mut value = 0u32;
        // SAFETY: value holds the 4 bytes `size` says.
        let e = unsafe {
            RegQueryValueExW(
                self.0,
                &name_w,
                None,
                Some(&raw mut kind),
                Some((&raw mut value).cast()),
                Some(&raw mut size),
            )
        };
        check(e, "cannot read registry value", name)?;
        Ok((kind == REG_DWORD).then_some(value))
    }

    /// `None` when the value does not exist or is not a string.
    pub fn text(&self, name: &str) -> Result<Option<Text>, String> {
        let name_w = HSTRING::from(name);
        let mut kind = REG_VALUE_TYPE::default();
        let mut size = 0u32;
        // SAFETY: size query with null data, out pointers valid.
        let e = unsafe {
            RegQueryValueExW(
                self.0,
                &name_w,
                None,
                Some(&raw mut kind),
                None,
                Some(&raw mut size),
            )
        };
        if e == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(e, "cannot read registry value", name)?;
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return Ok(None);
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        // SAFETY: buf holds `size` bytes.
        let e = unsafe {
            RegQueryValueExW(
                self.0,
                &name_w,
                None,
                Some(&raw mut kind),
                Some(buf.as_mut_ptr().cast()),
                Some(&raw mut size),
            )
        };
        check(e, "cannot read registry value", name)?;
        buf.truncate((size as usize) / 2);
        while buf.last() == Some(&0) {
            buf.pop();
        }
        Ok(Some(Text {
            value: String::from_utf16_lossy(&buf),
            expand: kind == REG_EXPAND_SZ,
        }))
    }

    pub fn set_text(&self, name: &str, value: &str, expand: bool) -> Result<(), String> {
        let wide: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
        let bytes: Vec<u8> = wide.iter().flat_map(|w| w.to_le_bytes()).collect();
        let kind = if expand { REG_EXPAND_SZ } else { REG_SZ };
        // SAFETY: bytes is a nul-terminated UTF-16 buffer.
        let e = unsafe { RegSetValueExW(self.0, &HSTRING::from(name), None, kind, Some(&bytes)) };
        check(e, "cannot write registry value", name)
    }

    /// removes one value; missing is fine.
    pub fn delete_value(&self, name: &str) -> Result<(), String> {
        // SAFETY: the key is open and the name lives across the call.
        let e = unsafe { RegDeleteValueW(self.0, &HSTRING::from(name)) };
        if e == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(e, "cannot delete registry value", name)
    }

    pub fn set_dword(&self, name: &str, value: u32) -> Result<(), String> {
        // SAFETY: 4 bytes for REG_DWORD.
        let e = unsafe {
            RegSetValueExW(
                self.0,
                &HSTRING::from(name),
                None,
                REG_DWORD,
                Some(&value.to_le_bytes()),
            )
        };
        check(e, "cannot write registry value", name)
    }
}

/// deletes `path` and everything under it. missing is fine.
pub fn delete_tree(hive: HKEY, path: &str) -> Result<(), String> {
    // SAFETY: path is a valid wide string.
    let e = unsafe { RegDeleteTreeW(hive, &HSTRING::from(path)) };
    if e == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(e, "cannot delete registry key", path)
}

fn hive_of(machine: bool) -> HKEY {
    if machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    }
}

/// the value `name` in `key`: a `REG_DWORD` as a number, a string as
/// text. `None` when it or the key is missing.
pub fn value(machine: bool, key: &str, name: &str) -> Result<Option<serde_json::Value>, String> {
    let Some(key) = Key::open_read(hive_of(machine), key)? else {
        return Ok(None);
    };
    if let Some(n) = key.dword(name)? {
        return Ok(Some(n.into()));
    }
    Ok(key.text(name)?.map(|t| t.value.into()))
}

/// writes the value `name` in `key`, making the key when missing: a
/// number as a `REG_DWORD`, true and false as 1 and 0, text as a string.
/// `None` deletes it.
pub fn set_value(
    machine: bool,
    key: &str,
    name: &str,
    value: Option<&serde_json::Value>,
) -> Result<(), String> {
    let Some(value) = value else {
        return match Key::open(hive_of(machine), key)? {
            Some(k) => k.delete_value(name),
            None => Ok(()),
        };
    };
    let k = Key::create(hive_of(machine), key)?;
    match value {
        serde_json::Value::String(s) => k.set_text(name, s, false),
        serde_json::Value::Bool(b) => k.set_dword(name, u32::from(*b)),
        serde_json::Value::Number(n) => {
            let n = n
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| format!("{name}: {n} does not fit a registry DWORD"))?;
            k.set_dword(name, n)
        }
        other => Err(format!("{name}: {other} cannot go in the registry")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_string_and_a_number_both_read_back() {
        let key = format!("Software\\kago-test-registry-{}", std::process::id());
        let long = "\"C:\\Program Files\\DLP Agent/dlp-agent.exe\" --tray";
        set_value(false, &key, "text", Some(&long.into())).unwrap();
        set_value(false, &key, "short", Some(&"ab".into())).unwrap();
        set_value(false, &key, "number", Some(&7.into())).unwrap();
        let read = |name| value(false, &key, name).unwrap();
        assert_eq!(read("text"), Some(long.into()));
        assert_eq!(read("short"), Some("ab".into()));
        assert_eq!(read("number"), Some(7.into()));
        assert_eq!(read("missing"), None);
        delete_tree(HKEY_CURRENT_USER, &key).unwrap();
    }
}
