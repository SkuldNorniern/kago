//! values in a JSON settings file, like another program's config. every
//! field kago does not touch stays as it was.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde_json::{Map, Value};

/// the value at `key` in `file`; `None` when the file or key is missing.
///
/// # Errors
///
/// when the file cannot be read or is not a JSON object.
pub fn value(file: &Path, key: &[String]) -> Result<Option<Value>, String> {
    let Some(root) = read(file)? else {
        return Ok(None);
    };
    let mut at = &Value::Object(root);
    for name in key {
        match at.get(name) {
            Some(next) => at = next,
            None => return Ok(None),
        }
    }
    Ok(Some(at.clone()))
}

/// sets `key` in `file` to `value`, or removes it for `None`. a missing
/// file is created, and so are the objects on the way to `key`.
///
/// # Errors
///
/// when the file cannot be read or written, or something on the way to
/// `key` is not an object.
pub fn set(file: &Path, key: &[String], value: Option<&Value>) -> Result<(), String> {
    let exists = file.exists();
    if value.is_none() && !exists {
        return Ok(());
    }
    let mut root = read(file)?.unwrap_or_default();
    let (last, path) = key.split_last().ok_or("an empty key")?;
    let Some(parent) = object_at(&mut root, path, file, value.is_some())? else {
        return Ok(());
    };
    match value {
        Some(v) => {
            parent.insert(last.clone(), v.clone());
        }
        None => {
            if parent.shift_remove(last).is_some() {
                prune(&mut root, key);
            }
        }
    }
    write(file, &root)
}

/// adds `item` to the list at `key`. `false` when it was there already,
/// so the install does not own it.
///
/// # Errors
///
/// as [`set`], or when `key` holds something other than a list.
pub fn add(file: &Path, key: &[String], item: &Value) -> Result<bool, String> {
    let mut root = read(file)?.unwrap_or_default();
    let list = list_at(&mut root, key, file, true)?.expect("created");
    if list.iter().any(|v| same(v, item)) {
        return Ok(false);
    }
    list.push(item.clone());
    write(file, &root)?;
    Ok(true)
}

/// takes `item` out of the list at `key`. missing is fine.
///
/// # Errors
///
/// as [`add`].
pub fn remove(file: &Path, key: &[String], item: &Value) -> Result<(), String> {
    let Some(mut root) = read(file)? else {
        return Ok(());
    };
    let Some(list) = list_at(&mut root, key, file, false)? else {
        return Ok(());
    };
    let before = list.len();
    list.retain(|v| !same(v, item));
    if list.len() != before {
        prune(&mut root, key);
        write(file, &root)?;
    }
    Ok(())
}

/// drops what `key` leads to once it is empty: the list or object at the
/// end, then each object on the way, deepest first. the root stays.
fn prune(map: &mut Map<String, Value>, key: &[String]) {
    let Some((first, rest)) = key.split_first() else {
        return;
    };
    let Some(child) = map.get_mut(first) else {
        return;
    };
    if !rest.is_empty() {
        match child {
            Value::Object(inner) => prune(inner, rest),
            _ => return,
        }
    }
    let empty = match child {
        Value::Object(inner) => inner.is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    };
    if empty {
        map.shift_remove(first);
    }
}

/// the object holding the last name of a key, made on the way when
/// `create`; `None` when it is missing and not made.
fn object_at<'a>(
    root: &'a mut Map<String, Value>,
    path: &[String],
    file: &Path,
    create: bool,
) -> Result<Option<&'a mut Map<String, Value>>, String> {
    let mut at = root;
    for name in path {
        if !at.contains_key(name) {
            if !create {
                return Ok(None);
            }
            at.insert(name.clone(), Value::Object(Map::new()));
        }
        at = at
            .get_mut(name)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("{}: `{name}` is not an object", file.display()))?;
    }
    Ok(Some(at))
}

fn list_at<'a>(
    root: &'a mut Map<String, Value>,
    key: &[String],
    file: &Path,
    create: bool,
) -> Result<Option<&'a mut Vec<Value>>, String> {
    let (last, path) = key.split_last().ok_or("an empty key")?;
    let Some(parent) = object_at(root, path, file, create)? else {
        return Ok(None);
    };
    if !create && !parent.contains_key(last) {
        return Ok(None);
    }
    let found = parent
        .entry(last.clone())
        .or_insert_with(|| Value::Array(Vec::new()));
    if found.is_null() {
        *found = Value::Array(Vec::new());
    }
    found
        .as_array_mut()
        .map(Some)
        .ok_or_else(|| format!("{}: `{}` is not a list", file.display(), key.join(".")))
}

fn read(file: &Path) -> Result<Option<Map<String, Value>>, String> {
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match serde_json::from_str(text) {
        Ok(Value::Object(map)) => Ok(Some(map)),
        Ok(_) => Err(format!("{}: not a JSON object", file.display())),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

/// the same item, where a path written with other slashes or, on
/// Windows, other case counts as the same: programs reading lists of
/// folders match them that way.
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_str(), b.as_str()) {
        (Some(a), Some(b)) => {
            let norm = |s: &str| {
                let s = s.replace('\\', "/");
                let s = s.trim_end_matches('/').to_owned();
                if cfg!(windows) { s.to_lowercase() } else { s }
            };
            norm(a) == norm(b)
        }
        _ => a == b,
    }
}

/// written next to the file and renamed over it, so a program reading it
/// never sees half a file.
fn write(file: &Path, root: &Map<String, Value>) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut text = serde_json::to_string_pretty(root).map_err(|e| e.to_string())?;
    text.push('\n');
    // renaming over a symlink would replace the link, so write to its target.
    let target = if fs::symlink_metadata(file).is_ok_and(|m| m.file_type().is_symlink()) {
        fs::canonicalize(file).map_err(|e| format!("{}: {e}", file.display()))?
    } else {
        file.to_path_buf()
    };
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(".kago");
    let tmp = std::path::PathBuf::from(tmp);
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &target).map_err(|e| format!("{}: {e}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kago-json-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn key(k: &str) -> Vec<String> {
        k.split('.').map(str::to_owned).collect()
    }

    fn read_back(file: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap()
    }

    #[test]
    fn add_keeps_other_fields_and_items() {
        let dir = temp("keep");
        let file = dir.join("openvrpaths.vrpath");
        fs::write(
            &file,
            r#"{"config":["C:\\Steam\\config"],"external_drivers":["C:\\other"],"version":1}"#,
        )
        .unwrap();
        let item = Value::from(dir.join("aira").display().to_string());
        let drivers = key("external_drivers");
        assert!(add(&file, &drivers, &item).unwrap());
        assert!(
            !add(&file, &drivers, &item).unwrap(),
            "second add owns nothing"
        );
        let v = read_back(&file);
        assert_eq!(v["config"][0], "C:\\Steam\\config");
        assert_eq!(v["external_drivers"].as_array().unwrap().len(), 2);

        remove(&file, &drivers, &item).unwrap();
        let v = read_back(&file);
        assert_eq!(v["external_drivers"], serde_json::json!(["C:\\other"]));
        assert_eq!(v["version"], 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn set_makes_the_way_and_removal_is_quiet() {
        let dir = temp("set");
        let file = dir.join("sub").join("settings.json");
        set(&file, &key("a.b"), None).unwrap();
        remove(&file, &key("list"), &Value::from(1)).unwrap();
        assert!(!file.exists());
        set(&file, &key("a.b"), Some(&Value::from("x"))).unwrap();
        assert_eq!(value(&file, &key("a.b")).unwrap(), Some(Value::from("x")));
        assert_eq!(value(&file, &key("a.c")).unwrap(), None);
        set(&file, &key("a.b"), None).unwrap();
        assert_eq!(read_back(&file), serde_json::json!({}));
        assert!(set(&file, &key("a.b.c"), Some(&Value::from(1))).is_ok());
        assert!(
            set(&file, &key("a.b.c.d"), Some(&Value::from(1))).is_err(),
            "1 is not an object"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn key_order_survives_edits() {
        let dir = temp("order");
        let file = dir.join("settings.json");
        fs::write(
            &file,
            r#"{"zeta":1,"mid":{"b":1,"a":2},"alpha":[],"beta":3}"#,
        )
        .unwrap();
        let order =
            |file: &Path| -> Vec<String> { read(file).unwrap().unwrap().keys().cloned().collect() };
        let expected = ["zeta", "mid", "alpha", "beta"];

        set(&file, &key("mid.c"), Some(&Value::from(3))).unwrap();
        assert_eq!(order(&file), expected);
        assert!(add(&file, &key("alpha"), &Value::from("x")).unwrap());
        assert_eq!(order(&file), expected);
        remove(&file, &key("alpha"), &Value::from("x")).unwrap();
        assert_eq!(order(&file), ["zeta", "mid", "beta"], "alpha is empty now");
        set(&file, &key("mid.b"), None).unwrap();
        let mid = value(&file, &key("mid")).unwrap().unwrap();
        let inner: Vec<_> = mid.as_object().unwrap().keys().cloned().collect();
        assert_eq!(inner, ["a", "c"]);
        set(&file, &key("zeta"), None).unwrap();
        assert_eq!(order(&file), ["mid", "beta"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn emptied_objects_on_the_way_go_but_others_stay() {
        let dir = temp("prune");
        let file = dir.join("settings.json");
        fs::write(
            &file,
            r#"{"empty":{},"a":{"b":{"c":1},"d":2},"l":{"items":["x"]}}"#,
        )
        .unwrap();
        set(&file, &key("a.b.c"), None).unwrap();
        assert_eq!(
            read_back(&file),
            serde_json::json!({"empty": {}, "a": {"d": 2}, "l": {"items": ["x"]}})
        );
        remove(&file, &key("l.items"), &Value::from("x")).unwrap();
        assert_eq!(
            read_back(&file),
            serde_json::json!({"empty": {}, "a": {"d": 2}})
        );
        set(&file, &key("a.d"), None).unwrap();
        assert_eq!(read_back(&file), serde_json::json!({"empty": {}}));
        set(&file, &key("nothing.here"), None).unwrap();
        assert_eq!(
            read_back(&file),
            serde_json::json!({"empty": {}}),
            "a missing key prunes nothing"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn slashes_and_trailing_separators_match() {
        assert!(same(&"C:\\a\\aira\\".into(), &"C:/a/aira".into()));
        assert!(!same(&"C:\\a\\aira2".into(), &"C:/a/aira".into()));
        assert!(!same(&"1".into(), &1.into()));
    }

    #[test]
    fn broken_file_is_an_error_not_overwritten() {
        let dir = temp("broken");
        let file = dir.join("settings.json");
        fs::write(&file, "{ not json").unwrap();
        assert!(add(&file, &key("x"), &Value::from(1)).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "{ not json");
        fs::remove_dir_all(dir).unwrap();
    }
}
