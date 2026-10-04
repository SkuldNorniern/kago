//! SteamVR's driver list: `external_drivers` in `openvrpaths.vrpath`, the
//! file `vrpathreg adddriver` writes. every other field stays as it was.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde_json::{Map, Value};

/// adds `driver` to the list in `file`. `false` when it was already there.
/// a missing file is created the way `vrpathreg` would.
///
/// # Errors
///
/// when the file cannot be read, is not a JSON object, or cannot be written.
pub fn add(file: &Path, driver: &Path) -> Result<bool, String> {
    let mut paths = read(file)?;
    let drivers = drivers(&mut paths, file)?;
    if drivers.iter().any(|d| same(d, driver)) {
        return Ok(false);
    }
    drivers.push(Value::String(driver.display().to_string()));
    write(file, &paths)?;
    Ok(true)
}

/// removes `driver` from the list in `file`. missing is fine.
///
/// # Errors
///
/// when the file cannot be read, is not a JSON object, or cannot be written.
pub fn remove(file: &Path, driver: &Path) -> Result<(), String> {
    if !file.exists() {
        return Ok(());
    }
    let mut paths = read(file)?;
    let drivers = drivers(&mut paths, file)?;
    let before = drivers.len();
    drivers.retain(|d| !same(d, driver));
    if drivers.len() != before {
        write(file, &paths)?;
    }
    Ok(())
}

/// whether `file` names a SteamVR runtime, which SteamVR writes there
/// once it has run.
#[must_use]
pub fn runtime_installed(file: &Path) -> bool {
    let Ok(text) = fs::read_to_string(file) else {
        return false;
    };
    serde_json::from_str::<Value>(&text).is_ok_and(|v| {
        v["runtime"].as_array().is_some_and(|r| {
            r.iter()
                .filter_map(Value::as_str)
                .any(|p| Path::new(p).is_dir())
        })
    })
}

/// whether `driver` is in the list in `file`.
#[cfg(test)]
fn contains(file: &Path, driver: &Path) -> Result<bool, String> {
    let mut paths = read(file)?;
    Ok(drivers(&mut paths, file)?.iter().any(|d| same(d, driver)))
}

fn read(file: &Path) -> Result<Map<String, Value>, String> {
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let mut fresh = Map::new();
            fresh.insert("jsonid".into(), "vrpathreg".into());
            fresh.insert("version".into(), 1.into());
            return Ok(fresh);
        }
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    match serde_json::from_str(&text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(format!("{}: not a JSON object", file.display())),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

fn drivers<'a>(
    paths: &'a mut Map<String, Value>,
    file: &Path,
) -> Result<&'a mut Vec<Value>, String> {
    let list = paths
        .entry("external_drivers")
        .or_insert_with(|| Value::Array(Vec::new()));
    if list.is_null() {
        *list = Value::Array(Vec::new());
    }
    list.as_array_mut()
        .ok_or_else(|| format!("{}: `external_drivers` is not a list", file.display()))
}

/// SteamVR matches folders without minding case or slash direction on
/// Windows, so the same folder written another way counts as the same.
fn same(entry: &Value, driver: &Path) -> bool {
    let Some(entry) = entry.as_str() else {
        return false;
    };
    let norm = |s: &str| {
        let s = s.replace('\\', "/");
        let s = s.trim_end_matches('/').to_owned();
        if cfg!(windows) { s.to_lowercase() } else { s }
    };
    norm(entry) == norm(&driver.display().to_string())
}

/// written next to the file and renamed over it, so SteamVR never reads
/// half a file.
fn write(file: &Path, paths: &Map<String, Value>) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut text = serde_json::to_string_pretty(paths).map_err(|e| e.to_string())?;
    text.push('\n');
    let tmp = file.with_extension("vrpath.kago");
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, file).map_err(|e| format!("{}: {e}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kago-openvr-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn add_keeps_other_fields_and_drivers() {
        let dir = temp("keep");
        let file = dir.join("openvrpaths.vrpath");
        fs::write(
            &file,
            r#"{"config":["C:\\Steam\\config"],"external_drivers":["C:\\other"],"runtime":["C:\\SteamVR"],"version":1}"#,
        )
        .unwrap();
        let driver = dir.join("aira");
        assert!(add(&file, &driver).unwrap());
        assert!(!add(&file, &driver).unwrap(), "second add owns nothing");
        let v: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["runtime"][0], "C:\\SteamVR");
        assert_eq!(v["external_drivers"].as_array().unwrap().len(), 2);
        assert!(contains(&file, &driver).unwrap());

        remove(&file, &driver).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["external_drivers"], serde_json::json!(["C:\\other"]));
        assert_eq!(v["config"][0], "C:\\Steam\\config");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_file_is_created_and_removal_is_quiet() {
        let dir = temp("missing");
        let file = dir.join("openvr").join("openvrpaths.vrpath");
        remove(&file, Path::new("/x")).unwrap();
        assert!(!file.exists());
        assert!(add(&file, Path::new("/x/aira")).unwrap());
        let v: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["jsonid"], "vrpathreg");
        assert_eq!(v["external_drivers"], serde_json::json!(["/x/aira"]));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn runtime_counts_only_when_its_folder_exists() {
        let dir = temp("runtime");
        let file = dir.join("openvrpaths.vrpath");
        assert!(!runtime_installed(&file), "no file");
        let runtime = dir.join("SteamVR");
        let json = serde_json::json!({ "runtime": [runtime.display().to_string()] });
        fs::write(&file, json.to_string()).unwrap();
        assert!(!runtime_installed(&file), "named but gone");
        fs::create_dir_all(&runtime).unwrap();
        assert!(runtime_installed(&file));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn slashes_and_trailing_separators_match() {
        assert!(same(&"C:\\a\\aira\\".into(), Path::new("C:/a/aira")));
        assert!(!same(&"C:\\a\\aira2".into(), Path::new("C:/a/aira")));
    }

    #[test]
    fn broken_file_is_an_error_not_overwritten() {
        let dir = temp("broken");
        let file = dir.join("openvrpaths.vrpath");
        fs::write(&file, "{ not json").unwrap();
        assert!(add(&file, Path::new("/x")).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "{ not json");
        fs::remove_dir_all(dir).unwrap();
    }
}
