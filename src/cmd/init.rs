//! `kago init`: a starter Kagofile for the project in a folder, filled in
//! from its Cargo.toml when there is one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kago::manifest::FILE_NAME;

use super::args::Args;

/// what a Cargo.toml says about the package, read without a TOML parser:
/// `key = "value"` lines of `[package]` and the names of `[[bin]]` tables.
#[derive(Debug, Default, PartialEq, Eq)]
struct Crate {
    name: Option<String>,
    description: Option<String>,
    homepage: Option<String>,
    bins: Vec<String>,
}

fn read_crate(text: &str) -> Crate {
    let mut out = Crate::default();
    let mut table = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            line.clone_into(&mut table);
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.split('#').next().unwrap_or("").trim();
        let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
            continue;
        };
        let value = value.to_owned();
        match (table.as_str(), key.trim()) {
            ("[package]", "name") => out.name = Some(value),
            ("[package]", "description") => out.description = Some(value),
            ("[package]", "homepage") => out.homepage = Some(value),
            ("[package]", "repository") if out.homepage.is_none() => out.homepage = Some(value),
            ("[[bin]]", "name") => out.bins.push(value),
            _ => {}
        }
    }
    if out.bins.is_empty()
        && let Some(name) = &out.name
    {
        out.bins.push(name.clone());
    }
    out
}

/// `aira-studio` as `Aira Studio`.
fn display_name(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or_else(String::new, |f| {
                f.to_uppercase().collect::<String>() + c.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// the first `.ico` near the top of the project, skipping build output.
fn find_icon(dir: &Path) -> Option<PathBuf> {
    let mut todo = vec![(dir.to_path_buf(), 0)];
    while let Some((folder, depth)) = todo.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if depth < 3
                    && !matches!(name, "target" | "node_modules" | "dist" | ".git" | ".kago")
                {
                    todo.push((path, depth + 1));
                }
            } else if name.to_ascii_lowercase().ends_with(".ico") {
                return path.strip_prefix(dir).ok().map(Path::to_path_buf);
            }
        }
    }
    None
}

fn license_file(dir: &Path) -> Option<&'static str> {
    [
        "LICENSE",
        "LICENSE.txt",
        "LICENSE.md",
        "LICENSE-MIT",
        "COPYING",
    ]
    .into_iter()
    .find(|name| dir.join(name).is_file())
}

/// a Kagofile for the project in `dir`, with `id` as the package id.
fn kagofile(dir: &Path, id: &str) -> String {
    let cargo = fs::read_to_string(dir.join("Cargo.toml")).ok();
    let krate = cargo.as_deref().map(read_crate).unwrap_or_default();
    let fallback = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("app")
        .to_owned();
    let crate_name = krate.name.clone().unwrap_or(fallback);
    let mut out = String::new();
    let mut line = |text: &str| {
        out.push_str(text);
        out.push('\n');
    };
    line("# made by `kago init`. check it with `kago check`, build the setup with");
    line("# `kago build`. every line is explained in kago's README.");
    line(&format!("package {id}"));
    line(&format!("  name {}", display_name(&crate_name)));
    if cargo.is_some() {
        line("  version from cargo");
    } else {
        line("  version 0.1.0");
    }
    line("  publisher Your Name");
    if let Some(description) = &krate.description {
        line(&format!("  description {description}"));
    }
    if let Some(homepage) = &krate.homepage {
        line(&format!("  homepage {homepage}"));
    }
    line("");
    if let Some(license) = license_file(dir) {
        // shown before installing; add `accept` to make people agree to it.
        line(&format!("license {license}"));
        line("");
    }
    let bins = if krate.bins.is_empty() {
        vec![crate_name]
    } else {
        krate.bins
    };
    let icon = find_icon(dir);
    for (i, bin) in bins.iter().enumerate() {
        line(&format!("app {bin}"));
        line(&format!("  windows target/release/{bin}.exe"));
        line(&format!("  linux target/release/{bin}"));
        if i == 0
            && let Some(icon) = &icon
        {
            line(&format!(
                "  icon windows {}",
                icon.display().to_string().replace('\\', "/")
            ));
        }
        line("");
    }
    line("# a Start menu entry for the first app.");
    line("shortcut");
    line("");
    line("# more you can add:");
    line("#   path optional \"Add to PATH\"        tick box that puts the app on PATH");
    line("#   component docs \"Documentation\" off  then indented `copy docs` lines");
    line("#   brand                               then `text`, `accent`, `side`, `header` lines");
    line("#   setup native                        under package: the classic NSIS-style look");
    out
}

/// writes a starter Kagofile into the folder given (default: here).
pub fn run(args: &Args) -> Result<ExitCode, String> {
    args.at_most_one()?;
    let dir = PathBuf::from(args.first().unwrap_or("."));
    let path = dir.join(FILE_NAME);
    if path.exists() && !args.flag("--force") {
        return Err(format!(
            "{} already exists; `--force` replaces it",
            path.display()
        ));
    }
    let id = args.value("--id").map_or_else(
        || {
            let name = fs::read_to_string(dir.join("Cargo.toml"))
                .ok()
                .and_then(|t| read_crate(&t).name)
                .unwrap_or_else(|| "app".to_owned());
            format!(
                "com.example.{}",
                name.to_ascii_lowercase().replace('_', "-")
            )
        },
        str::to_owned,
    );
    let text = kagofile(&dir, &id);
    fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("wrote {}", path.display());
    if args.value("--id").is_none() {
        println!("  set your own package id on the first line, like dev.yourname.app");
    }
    println!("next:");
    println!("  cargo build --release");
    println!("  kago check        is the Kagofile right?");
    println!("  kago build        make the setup in dist/");
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kago::manifest::Manifest;

    #[test]
    fn reads_package_and_bins() {
        let c = read_crate(
            "[package]\nname = \"aira-studio\" # app\ndescription = \"Set up trackers\"\nrepository = \"https://x/y\"\n\n[dependencies]\nname = \"not this\"\n\n[[bin]]\nname = \"aira\"\npath = \"src/main.rs\"\n",
        );
        assert_eq!(c.name.as_deref(), Some("aira-studio"));
        assert_eq!(c.description.as_deref(), Some("Set up trackers"));
        assert_eq!(c.homepage.as_deref(), Some("https://x/y"));
        assert_eq!(c.bins, ["aira"]);
        assert_eq!(display_name("aira-studio"), "Aira Studio");
    }

    #[test]
    fn the_starter_kagofile_checks_clean() {
        let dir = std::env::temp_dir().join(format!("kago-init-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("packaging")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"ozone\"\nversion = \"0.3.1\"\ndescription = \"Text editor\"\n",
        )
        .unwrap();
        fs::write(dir.join("LICENSE"), "MIT").unwrap();
        fs::write(dir.join("packaging/ozone.ico"), b"").unwrap();
        let text = kagofile(&dir, "dev.test.ozone");
        let (m, d) = Manifest::parse_in("Kagofile", &text, &dir);
        assert!(!d.has_errors(), "{d:?}\n{text}");
        let m = m.unwrap();
        assert_eq!(m.package.name, "Ozone");
        assert_eq!(m.package.version.to_string(), "0.3.1");
        assert!(text.contains("icon windows packaging/ozone.ico"), "{text}");
        assert!(text.contains("license LICENSE"));
        fs::remove_dir_all(dir).unwrap();
    }
}
