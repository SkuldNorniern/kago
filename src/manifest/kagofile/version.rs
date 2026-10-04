//! `version 1.2.0`, `version from cargo [file]`, `version from file <file>`,
//! `version from git`.

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
        ["from", "git"] => git(base),
        _ => Err("expected `version <x.y.z>` or `version from cargo|file <path>|git`".to_owned()),
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
        std::fs::remove_dir_all(dir).unwrap();
    }
}
