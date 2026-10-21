//! folders Windows and other programs keep, which removing an app's data
//! must never take.

use std::path::{Path, PathBuf};

/// folders named by environment variables, like `%APPDATA%`.
const KNOWN: &[&str] = &[
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ALLUSERSPROFILE",
    "PUBLIC",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "CommonProgramFiles(x86)",
    "SystemRoot",
    "windir",
    "TEMP",
    "TMP",
    "OneDrive",
    "OneDriveConsumer",
    "OneDriveCommercial",
];

/// folders right inside those that hold more than one app's things.
const INSIDE: &[&str] = &[
    "3d objects",
    "appdata",
    "application data",
    "comms",
    "connecteddevicesplatform",
    "contacts",
    "crashdumps",
    "d3dscache",
    "desktop",
    "documents",
    "downloads",
    "favorites",
    "kago",
    "links",
    "local",
    "locallow",
    "microsoft",
    "music",
    "onedrive",
    "package cache",
    "packages",
    "pictures",
    "programs",
    "publishers",
    "roaming",
    "saved games",
    "searches",
    "start menu",
    "temp",
    "templates",
    "videos",
    "virtualstore",
];

fn lower(p: &Path) -> String {
    p.display()
        .to_string()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// whether `dir` is one of those folders, holds one, or sits right inside
/// one under a shared name.
pub fn shared(dir: &Path) -> bool {
    let known: Vec<String> = KNOWN
        .iter()
        .filter_map(std::env::var_os)
        .map(|v| lower(&PathBuf::from(v)))
        .filter(|k| !k.is_empty())
        .collect();
    let at = lower(dir);
    let holds = |k: &String| *k == at || k.starts_with(&format!("{at}\\"));
    if at.is_empty() || known.iter().any(holds) {
        return true;
    }
    let in_known = dir.parent().is_some_and(|p| known.contains(&lower(p)));
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_lowercase);
    in_known && name.is_some_and(|n| INSIDE.contains(&n.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// reads environment variables only.
    #[test]
    fn known_folders_and_what_holds_them_are_shared() {
        let var = |v| PathBuf::from(std::env::var_os(v).unwrap());
        let local = var("LOCALAPPDATA");
        let home = var("USERPROFILE");
        assert!(shared(&local));
        assert!(shared(&home));
        assert!(shared(Path::new(r"C:\")));
        assert!(shared(&local.join("Programs")));
        assert!(shared(&local.join("Kago")));
        assert!(shared(&var("APPDATA").join("Microsoft")));
        assert!(shared(&home.join("Documents")));
        assert!(!shared(&local.join("Aira Studio")));
        assert!(!shared(&home.join("Documents").join("My Game")));
        assert!(!shared(&var("APPDATA").join("Aira")));
    }
}
