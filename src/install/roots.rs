use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::manifest::{Root, SemanticPath};

/// where each semantic root is on this machine for one install. filled by
/// the platform, recorded in the receipt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roots(pub BTreeMap<Root, PathBuf>);

impl Roots {
    /// absolute path for `path`.
    ///
    /// # Errors
    ///
    /// when the platform has no location for the root in this scope.
    pub fn resolve(&self, path: &SemanticPath) -> Result<PathBuf, String> {
        let mut out = self
            .0
            .get(&path.root)
            .ok_or_else(|| format!("{} has no location on this platform and scope", path.root))?
            .clone();
        out.extend(path.segments());
        Ok(out)
    }

    /// the same roots with `{app}` at `dir` and `{bin}` under it.
    #[must_use]
    pub fn with_app(mut self, dir: &Path) -> Self {
        if self.0.contains_key(&Root::Bin) {
            self.0.insert(Root::Bin, dir.join("bin"));
        }
        self.0.insert(Root::App, dir.to_path_buf());
        self
    }

    /// only the roots `paths` use, for the receipt.
    #[must_use]
    pub fn used<'a>(
        &self,
        paths: impl IntoIterator<Item = &'a SemanticPath>,
    ) -> BTreeMap<Root, PathBuf> {
        let mut out = BTreeMap::new();
        for p in paths {
            if let Some(dir) = self.0.get(&p.root) {
                out.insert(p.root, dir.clone());
            }
        }
        out
    }
}

/// why a folder cannot take an install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationError {
    /// relative paths depend on where setup was started.
    NotAbsolute,
    /// a drive or filesystem root, like `C:\` or `/`.
    Root,
    /// `.` or `..` in the path.
    Unclear,
    /// holds files that are not this package's.
    NotEmpty,
}

impl fmt::Display for LocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotAbsolute => "the folder must be a full path",
            Self::Root => "a drive root cannot hold an install",
            Self::Unclear => "the folder must not contain `.` or `..`",
            Self::NotEmpty => "the folder already has files in it; pick an empty or new folder",
        })
    }
}

/// checks a folder someone typed or picked for `{app}`. only the path; the
/// caller checks what is on disk.
///
/// # Errors
///
/// when the path cannot hold an install.
pub fn check_location(dir: &Path) -> Result<(), LocationError> {
    if !dir.is_absolute() {
        return Err(LocationError::NotAbsolute);
    }
    let mut named = 0;
    for part in dir.components() {
        match part {
            Component::Normal(_) => named += 1,
            Component::CurDir | Component::ParentDir => return Err(LocationError::Unclear),
            Component::Prefix(_) | Component::RootDir => {}
        }
    }
    if named == 0 {
        return Err(LocationError::Root);
    }
    Ok(())
}

/// [`check_location`], then that `dir` is new or empty, unless it is
/// where this package is installed now.
///
/// # Errors
///
/// when the path cannot hold an install or the folder has other files.
pub fn check_folder(dir: &Path, installed: Option<&Path>) -> Result<(), LocationError> {
    check_location(dir)?;
    if installed.is_some_and(|now| now == dir) {
        return Ok(());
    }
    let has_files = std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some());
    if has_files {
        Err(LocationError::NotEmpty)
    } else {
        Ok(())
    }
}

/// a package name as one safe directory name. drops characters that are
/// not portable, never empty.
#[must_use]
pub fn folder_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|c| {
            !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') && !c.is_control()
        })
        .collect();
    let clean = clean.trim().trim_end_matches('.').to_owned();
    if clean.is_empty() || clean == "." || clean == ".." {
        "app".to_owned()
    } else {
        clean
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_must_be_full_named_paths() {
        let abs = if cfg!(windows) {
            r"C:\Apps\Ozone"
        } else {
            "/opt/ozone"
        };
        assert_eq!(check_location(Path::new(abs)), Ok(()));
        assert_eq!(
            check_location(Path::new("Apps/Ozone")),
            Err(LocationError::NotAbsolute)
        );
        let root = if cfg!(windows) { r"C:\" } else { "/" };
        assert_eq!(check_location(Path::new(root)), Err(LocationError::Root));
        let up = if cfg!(windows) {
            r"C:\Apps\..\Ozone"
        } else {
            "/opt/../ozone"
        };
        assert_eq!(check_location(Path::new(up)), Err(LocationError::Unclear));
    }

    #[test]
    fn with_app_moves_bin_along() {
        let roots = Roots(BTreeMap::from([
            (Root::App, PathBuf::from("/a")),
            (Root::Bin, PathBuf::from("/a/bin")),
            (Root::Data, PathBuf::from("/d")),
        ]))
        .with_app(Path::new("/x/App"));
        assert_eq!(roots.0[&Root::Bin], PathBuf::from("/x/App/bin"));
        assert_eq!(roots.0[&Root::Data], PathBuf::from("/d"));
    }

    #[test]
    fn resolves_under_root() {
        let roots = Roots(BTreeMap::from([(Root::App, PathBuf::from("/opt/x"))]));
        let p = roots.resolve(&"{app}/bin/x".parse().unwrap()).unwrap();
        assert_eq!(p, PathBuf::from("/opt/x/bin/x"));
        assert!(roots.resolve(&"{config}/a".parse().unwrap()).is_err());
    }

    #[test]
    fn folder_names_are_safe() {
        assert_eq!(folder_name("Aira Studio"), "Aira Studio");
        assert_eq!(folder_name("a/b:c"), "abc");
        assert_eq!(folder_name("..."), "app");
        assert_eq!(folder_name(""), "app");
    }
}
