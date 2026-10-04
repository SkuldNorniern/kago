//! installed receipt schema. what an install owns, read by uninstall, repair and update.
//!
//! a receipt is written only at commit. it is the authority for what kago
//! may remove later, never the current manifest.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::install::{Digest, Scope};
use crate::manifest::{Ownership, Root, SemanticPath};
use crate::package::{PackageId, Target, Version};

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub schema: u32,
    pub package: PackageId,
    pub name: String,
    pub version: Version,
    pub publisher: Option<String>,
    pub target: Target,
    pub scope: Scope,
    pub components: Vec<String>,
    /// absolute location each used root had at install time.
    pub roots: BTreeMap<Root, PathBuf>,
    pub resources: Vec<Resource>,
}

/// one thing the install created and owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Resource {
    Directory {
        path: SemanticPath,
    },
    File {
        path: SemanticPath,
        size: u64,
        sha256: Digest,
        ownership: Ownership,
    },
    /// exact entry added to PATH.
    PathEntry {
        entry: String,
        scope: Scope,
    },
    Shortcut {
        path: PathBuf,
    },
    /// uninstall entry, desktop file or similar platform registration.
    Registration {
        key: String,
        scope: Scope,
    },
}

impl Receipt {
    /// absolute path of `path` under the recorded roots.
    #[must_use]
    pub fn absolute(&self, path: &SemanticPath) -> Option<PathBuf> {
        let mut out = self.roots.get(&path.root)?.clone();
        out.extend(path.segments());
        Some(out)
    }

    /// pretty JSON, stable field order.
    ///
    /// # Panics
    ///
    /// never; every field serializes.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("receipt serializes")
    }

    /// # Errors
    ///
    /// when `json` is not a receipt or has another schema.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let receipt: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if receipt.schema != SCHEMA {
            return Err(format!(
                "receipt schema {} is not supported, this kago reads schema {SCHEMA}",
                receipt.schema
            ));
        }
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt() -> Receipt {
        Receipt {
            schema: SCHEMA,
            package: PackageId::new("dev.skuld.ozone").unwrap(),
            name: "Ozone".into(),
            version: Version::parse("0.1.0").unwrap(),
            publisher: None,
            target: "windows-x86_64".parse().unwrap(),
            scope: Scope::User,
            components: vec!["core".into()],
            roots: BTreeMap::from([(
                Root::App,
                PathBuf::from("C:/Users/x/AppData/Local/Programs/Ozone"),
            )]),
            resources: vec![
                Resource::Directory {
                    path: "{app}/themes".parse().unwrap(),
                },
                Resource::File {
                    path: "{app}/ozone.exe".parse().unwrap(),
                    size: 3,
                    sha256: Digest([7; 32]),
                    ownership: Ownership::Owned,
                },
                Resource::PathEntry {
                    entry: "C:/Users/x/AppData/Local/Programs/Ozone".into(),
                    scope: Scope::User,
                },
            ],
        }
    }

    #[test]
    fn json_round_trip() {
        let r = receipt();
        let json = r.to_json();
        assert!(json.contains("\"kind\": \"file\""), "{json}");
        assert!(json.contains("\"path\": \"{app}/ozone.exe\""), "{json}");
        assert!(json.contains("\"app\":"), "{json}");
        assert_eq!(Receipt::from_json(&json).unwrap(), r);
    }

    #[test]
    fn rejects_other_schema() {
        let json = receipt()
            .to_json()
            .replace("\"schema\": 1", "\"schema\": 9");
        assert!(Receipt::from_json(&json).unwrap_err().contains("schema 9"));
    }

    #[test]
    fn absolute_paths_use_recorded_roots() {
        let r = receipt();
        let p = r.absolute(&"{app}/themes/a.toml".parse().unwrap()).unwrap();
        assert!(p.ends_with("Ozone/themes/a.toml"));
        assert!(r.absolute(&"{config}/x".parse().unwrap()).is_none());
    }
}
