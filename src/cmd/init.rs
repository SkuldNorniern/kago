//! `kago init`: a starter Kagofile for the project in a folder, filled in
//! from its Cargo.toml, pyproject.toml or package.json when there is one.

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

/// what kago can tell about the project in a folder from its own files:
/// a Cargo.toml, a pyproject.toml or a package.json.
#[derive(Debug, Default, PartialEq, Eq)]
struct Project {
    name: Option<String>,
    description: Option<String>,
    homepage: Option<String>,
    /// the package's `version` line.
    version: String,
    /// `build` lines that make the files.
    build: Vec<String>,
    /// `app` blocks with the lines under them, and lines after them.
    apps: Vec<(String, Vec<String>)>,
    extra: Vec<String>,
}

/// `key = "value"` lines of one `[table]` in a TOML file.
fn toml_table(text: &str, wanted: &str) -> Vec<(String, String)> {
    let mut table = String::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            line.clone_into(&mut table);
            continue;
        }
        if table != wanted {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.split('#').next().unwrap_or("").trim();
        if let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            out.push((key.trim().to_owned(), value.to_owned()));
        }
    }
    out
}

fn rust(dir: &Path, text: &str) -> Project {
    let krate = read_crate(text);
    let fallback = folder(dir);
    let bins = if krate.bins.is_empty() {
        vec![krate.name.clone().unwrap_or(fallback)]
    } else {
        krate.bins.clone()
    };
    Project {
        name: krate.name,
        description: krate.description,
        homepage: krate.homepage,
        version: "from cargo".to_owned(),
        build: vec!["cargo build --release".to_owned()],
        apps: bins
            .into_iter()
            .map(|bin| {
                let lines = vec![
                    format!("windows target/release/{bin}.exe"),
                    format!("linux target/release/{bin}"),
                ];
                (bin, lines)
            })
            .collect(),
        extra: Vec::new(),
    }
}

/// a Python project, packed by `PyInstaller` into one folder: the program
/// and its `_internal` folder.
fn python(dir: &Path, text: &str) -> Project {
    let fields = toml_table(text, "[project]");
    let poetry = toml_table(text, "[tool.poetry]");
    let get = |key: &str| {
        fields
            .iter()
            .chain(&poetry)
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    let name = get("name").unwrap_or_else(|| folder(dir));
    let version = if fields.iter().any(|(k, _)| k == "version") {
        "from toml pyproject.toml project.version"
    } else if poetry.iter().any(|(k, _)| k == "version") {
        "from toml pyproject.toml tool.poetry.version"
    } else {
        "0.1.0"
    };
    let module = name.replace('-', "_");
    let entry = [
        format!("src/{module}/__main__.py"),
        format!("{module}/__main__.py"),
        "main.py".to_owned(),
        format!("{module}.py"),
    ]
    .into_iter()
    .find(|e| dir.join(e).is_file())
    .unwrap_or_else(|| "main.py".to_owned());
    Project {
        homepage: get("homepage").or_else(|| {
            toml_table(text, "[project.urls]")
                .first()
                .map(|(_, v)| v.clone())
        }),
        description: get("description"),
        version: version.to_owned(),
        build: vec![format!(
            "python -m PyInstaller --noconfirm --windowed --name {name} {entry}"
        )],
        apps: vec![(
            name.clone(),
            vec![format!("windows dist/{name}/{name}.exe")],
        )],
        extra: vec![format!("copy dist/{name}/_internal")],
        name: Some(name),
    }
}

/// a Node project. how it becomes a program differs (Electron, Tauri,
/// pkg), so the app path is a guess to fix.
fn node(dir: &Path, text: &str) -> Project {
    let json: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let text_at = |key: &str| json[key].as_str().map(str::to_owned);
    let name = text_at("name").unwrap_or_else(|| folder(dir));
    let name = name.rsplit('/').next().unwrap_or(&name).to_owned();
    let build = if json["scripts"]["build"].is_string() {
        // npm is npm.cmd on Windows, which only cmd starts.
        vec!["cmd /c npm run build".to_owned()]
    } else {
        Vec::new()
    };
    Project {
        description: text_at("description"),
        homepage: text_at("homepage"),
        version: if text_at("version").is_some() {
            "from json package.json version".to_owned()
        } else {
            "0.1.0".to_owned()
        },
        build,
        apps: vec![(
            name.clone(),
            vec![format!(
                "windows dist/{name}.exe   # where your build puts the program"
            )],
        )],
        extra: Vec::new(),
        name: Some(name),
    }
}

fn folder(dir: &Path) -> String {
    dir.canonicalize()
        .ok()
        .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "app".to_owned())
}

/// the project in `dir`, from the first project file found.
fn project(dir: &Path) -> Project {
    let read = |file: &str| fs::read_to_string(dir.join(file)).ok();
    if let Some(text) = read("Cargo.toml") {
        return rust(dir, &text);
    }
    if let Some(text) = read("pyproject.toml") {
        return python(dir, &text);
    }
    if let Some(text) = read("package.json") {
        return node(dir, &text);
    }
    let name = folder(dir);
    Project {
        version: "0.1.0".to_owned(),
        apps: vec![(
            name.clone(),
            vec![format!(
                "windows dist/{name}.exe   # where your build puts the program"
            )],
        )],
        name: Some(name),
        ..Project::default()
    }
}

/// a Kagofile for the project in `dir`, with `id` as the package id.
fn kagofile(dir: &Path, id: &str) -> String {
    let project = project(dir);
    let name = project.name.clone().unwrap_or_else(|| folder(dir));
    let mut out = String::new();
    let mut line = |text: &str| {
        out.push_str(text);
        out.push('\n');
    };
    line("# made by `kago init`. check it with `kago check`, build the setup with");
    line("# `kago build`. every line is explained in kago's README.");
    line(&format!("package {id}"));
    line(&format!("  name {}", display_name(&name)));
    line(&format!("  version {}", project.version));
    line("  publisher Your Name");
    if let Some(description) = &project.description {
        line(&format!("  description {description}"));
    }
    if let Some(homepage) = &project.homepage {
        line(&format!("  homepage {homepage}"));
    }
    line("");
    if !project.build.is_empty() {
        line("# `kago build` runs these first; `--no-build` skips them.");
        for b in &project.build {
            line(&format!("build {b}"));
        }
        line("");
    }
    if let Some(license) = license_file(dir) {
        // shown before installing; add `accept` to make people agree to it.
        line(&format!("license {license}"));
        line("");
    }
    let icon = find_icon(dir);
    for (i, (app, lines)) in project.apps.iter().enumerate() {
        line(&format!("app {app}"));
        for l in lines {
            line(&format!("  {l}"));
        }
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
    for l in &project.extra {
        line(l);
    }
    if !project.extra.is_empty() {
        line("");
    }
    line("# a Start menu entry for the first app.");
    line("shortcut");
    line("");
    line("# more you can add:");
    line("#   scope choose                        under package: per user or for everyone");
    line("#   path optional \"Add to PATH\"        tick box that puts the app on PATH");
    line("#   component docs \"Documentation\" off  then indented `copy docs` lines");
    line("#   json, registry, env                 settings put back at uninstall");
    line("#   run <program> [arguments]           with `undo ...` under it");
    line("#   action <name> <params>              your own statement, made of these");
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
            let name = project(&dir).name.unwrap_or_else(|| "app".to_owned());
            format!(
                "com.example.{}",
                name.to_ascii_lowercase().replace(['_', ' '], "-")
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
    println!("  kago check        is the Kagofile right?");
    println!("  kago build        run its build lines and make the setup in dist/");
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
        assert!(text.contains("build cargo build --release"), "{text}");
        fs::remove_dir_all(dir).unwrap();
    }

    fn starter(name: &str, file: &str, text: &str) -> (String, Manifest) {
        let dir = std::env::temp_dir().join(format!("kago-init-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(file), text).unwrap();
        let kagofile = kagofile(&dir, "dev.test.app");
        let (m, d) = Manifest::parse_in("Kagofile", &kagofile, &dir);
        assert!(!d.has_errors(), "{d:?}\n{kagofile}");
        fs::remove_dir_all(dir).unwrap();
        (kagofile, m.unwrap())
    }

    #[test]
    fn python_projects_build_with_pyinstaller() {
        let (text, m) = starter(
            "python",
            "pyproject.toml",
            "[project]\nname = \"tracker-tool\"\nversion = \"2.1.0\"\ndescription = \"Tracks\"\n",
        );
        assert_eq!(m.package.version.to_string(), "2.1.0");
        assert_eq!(m.package.name, "Tracker Tool");
        assert!(
            text.contains(
                "build python -m PyInstaller --noconfirm --windowed --name tracker-tool main.py"
            ),
            "{text}"
        );
        assert!(text.contains("windows dist/tracker-tool/tracker-tool.exe"));
        assert!(text.contains("copy dist/tracker-tool/_internal"));
    }

    #[test]
    fn node_projects_read_package_json() {
        let (text, m) = starter(
            "node",
            "package.json",
            r#"{"name":"@eira/viewer","version":"0.4.0","scripts":{"build":"vite build"}}"#,
        );
        assert_eq!(m.package.version.to_string(), "0.4.0");
        assert!(text.contains("app viewer"), "{text}");
        assert!(text.contains("build cmd /c npm run build"));
    }
}
