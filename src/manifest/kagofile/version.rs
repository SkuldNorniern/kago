//! `version 1.2.0`, or read from the project: `version from toml <file>
//! <key>` (like `pyproject.toml project.version`), `version from json
//! <file> <key>` (like `package.json version`), `version from file
//! <file>`, `version from git`, and `version from cargo [file]` for a
//! Cargo.toml.

use std::path::Path;
use std::process::Command;

use super::tree::Word;

/// the version text a `version` line names. not checked as SemVer here;
/// the validator does that and points at the line.
pub fn read(args: &[Word], base: &Path) -> Result<String, String> {
    let texts: Vec<&str> = args.iter().map(|w| w.text.as_str()).collect();
    match texts.as_slice() {
        [literal] => Ok((*literal).to_owned()),
        ["from", "cargo"] => cargo(&base.join("Cargo.toml")),
        ["from", "cargo", file] => cargo(&base.join(file)),
        ["from", "file", file] => {
            let path = base.join(file);
            std::fs::read_to_string(&path)
                .map(|s| s.trim().to_owned())
                .map_err(|e| format!("cannot read {}: {e}", path.display()))
        }
        ["from", "toml", file, key] => toml(&base.join(file), key),
        ["from", "json", file, key] => json(&base.join(file), key),
        ["from", "git"] => git(base),
        _ => Err(
            "expected `version <x.y.z>` or `version from toml <file> <key>|json <file> <key>|file <file>|git|cargo`"
                .to_owned(),
        ),
    }
}

/// `version = "..."` from the `[package]` table of a Cargo.toml.
fn cargo(path: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(value) = line.strip_prefix("version") {
            let value = value.trim_start();
            if value.starts_with(".workspace") {
                return Err(format!(
                    "{} takes its version from the workspace; point `from cargo` at the workspace Cargo.toml",
                    path.display()
                ));
            }
            if let Some(v) = value.strip_prefix('=') {
                let v = v.split('#').next().unwrap_or("").trim().trim_matches('"');
                return Ok(v.to_owned());
            }
        }
    }
    Err(format!(
        "no `version` in the [package] of {}",
        path.display()
    ))
}

/// the text at a dotted `key` like `project.version` in a TOML file: a
/// `key = "text"` line under its `[table]` header. enough for the project
/// files versions live in, without a TOML parser.
fn toml(path: &Path, key: &str) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut table = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('[') {
            header
                .trim_start_matches('[')
                .split(']')
                .next()
                .unwrap_or("")
                .trim()
                .clone_into(&mut table);
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim().trim_matches('"');
        let full = if table.is_empty() {
            name.to_owned()
        } else {
            format!("{table}.{name}")
        };
        if full != key {
            continue;
        }
        let value = value.trim();
        let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'');
        if let Some(q) = quote
            && let Some(end) = value[1..].find(q)
        {
            return Ok(value[1..=end].to_owned());
        }
        return Err(format!(
            "`{key}` in {} is not text in quotes",
            path.display()
        ));
    }
    Err(format!("no `{key}` in {}", path.display()))
}

/// the text at a dotted `key` like `version` in a JSON file.
fn json(path: &Path, key: &str) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    key.split('.')
        .try_fold(&value, |at, name| at.get(name))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("no text at `{key}` in {}", path.display()))
}

/// newest tag, without a leading `v`.
fn git(base: &Path) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(base)
        .args(["describe", "--tags", "--abbrev=0"])
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git describe found no tag: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let tag = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok(tag.strip_prefix('v').unwrap_or(&tag).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(texts: &[&str]) -> Vec<Word> {
        texts
            .iter()
            .map(|t| Word {
                text: (*t).to_owned(),
                at: 0,
                quoted: false,
            })
            .collect()
    }

    #[test]
    fn literal_and_cargo() {
        let dir = std::env::temp_dir().join(format!("kago-version-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[workspace]\nversion = \"9.9.9\"\n\n[package]\nname = \"x\"\nversion = \"1.4.2\" # here\n",
        )
        .unwrap();
        assert_eq!(read(&w(&["1.0.0"]), &dir).unwrap(), "1.0.0");
        assert_eq!(read(&w(&["from", "cargo"]), &dir).unwrap(), "1.4.2");
        std::fs::write(dir.join("VERSION"), "2.0.0\n").unwrap();
        assert_eq!(
            read(&w(&["from", "file", "VERSION"]), &dir).unwrap(),
            "2.0.0"
        );
        assert!(read(&w(&["from", "nowhere"]), &dir).is_err());
        std::fs::write(
            dir.join("pyproject.toml"),
            "[build-system]\nversion = \"0\"\n[project]\nname = \"x\"\nversion = '3.1.0'\n[tool.poetry]\nversion = \"9\"\n",
        )
        .unwrap();
        let toml = |key: &str| read(&w(&["from", "toml", "pyproject.toml", key]), &dir);
        assert_eq!(toml("project.version").unwrap(), "3.1.0");
        assert_eq!(toml("tool.poetry.version").unwrap(), "9");
        assert!(toml("project.nope").is_err());
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"x","version":"4.0.1","engines":{"node":"20"}}"#,
        )
        .unwrap();
        let json = |key: &str| read(&w(&["from", "json", "package.json", key]), &dir);
        assert_eq!(json("version").unwrap(), "4.0.1");
        assert_eq!(json("engines.node").unwrap(), "20");
        assert!(json("engines").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
