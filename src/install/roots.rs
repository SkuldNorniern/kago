use std::collections::BTreeMap;
use std::path::PathBuf;

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
