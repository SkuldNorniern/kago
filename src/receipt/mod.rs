//! installed receipt schema. what an install owns, read by uninstall, repair and update.
//!
//! written only at commit; the authority for what kago may remove later,
//! never the current manifest.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::install::{Digest, Scope, Slot};
use crate::manifest::{Edit, Ownership, Root, SemanticPath, Value};
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
    /// optional components there were to pick; those not in `components`
    /// were left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub offered: Vec<String>,
    /// how the setup looked, for the uninstaller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<crate::install::Look>,
    /// where `kago update` looks for newer versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<String>,
    /// what was filled in on the package's own pages, secrets left out,
    /// to start from in an update.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub answers: BTreeMap<String, String>,
    /// folders of the app's own data, where they were at install, for
    /// removing the person's data at uninstall when asked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub purge: Vec<PathBuf>,
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
        /// the person's own file that was there first, kept aside in
        /// kago's state and put back at uninstall.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replaced: Option<PathBuf>,
    },
    /// exact entry added to PATH.
    PathEntry {
        entry: String,
        scope: Scope,
    },
    Shortcut {
        path: PathBuf,
    },
    /// starts a program when the user signs in.
    LoginItem {
        name: String,
    },
    /// a Windows service.
    Service {
        name: String,
    },
    /// a Windows Firewall rule, in the package's group.
    FirewallRule {
        name: String,
        group: String,
    },
    /// a driver package in the driver store, by the name Windows gave it
    /// there, like `oem12.inf`.
    Driver {
        published: String,
    },
    /// a software device the install made for a driver.
    Device {
        instance: String,
        hardware_id: String,
    },
    /// the package's own program to run first at uninstall.
    UninstallHook {
        command: String,
        arguments: Vec<String>,
        /// a failure stops the uninstall before anything is removed.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        required: bool,
    },
    /// a value set outside the package. uninstall takes an added item
    /// out, and puts `previous` back where the value is still the one set.
    Changed {
        slot: Slot,
        edit: Edit,
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous: Option<Value>,
    },
    /// a JSON settings file the install created. uninstall removes it once
    /// nothing is left in it.
    SettingsFile {
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

    /// where every installed file is, for finding programs that use them.
    #[must_use]
    pub fn files(&self) -> Vec<PathBuf> {
        self.resources
            .iter()
            .filter_map(|r| match r {
                Resource::File { path, .. } => self.absolute(path),
                _ => None,
            })
            .collect()
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
        let mut value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if let Some(resources) = value["resources"].as_array_mut() {
            resources.iter_mut().for_each(steamvr_driver);
        }
        let receipt: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if receipt.schema != SCHEMA {
            return Err(format!(
                "receipt schema {} is not supported, this kago reads schema {SCHEMA}",
                receipt.schema
            ));
        }
        Ok(receipt)
    }
}

/// a SteamVR driver from a receipt older kagos wrote, when it was a
/// resource of its own: the item it added to SteamVR's list.
fn steamvr_driver(resource: &mut serde_json::Value) {
    if resource["kind"] != "steam-vr-driver" {
        return;
    }
    let Some(local) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let file = PathBuf::from(local).join(r"openvr\openvrpaths.vrpath");
    *resource = serde_json::json!({
        "kind": "changed",
        "slot": { "in": "json", "file": file, "key": ["external_drivers"] },
        "edit": "add",
        "value": resource["directory"].clone(),
    });
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
            offered: Vec::new(),
            look: None,
            update: None,
            answers: BTreeMap::new(),
            purge: Vec::new(),
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
                    replaced: None,
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
    #[cfg(windows)]
    fn an_older_steamvr_driver_reads_as_a_change() {
        let mut json: serde_json::Value = serde_json::from_str(&receipt().to_json()).unwrap();
        json["resources"] = serde_json::json!([
            { "kind": "steam-vr-driver", "directory": r"C:\x\steamvr\aira" }
        ]);
        let read = Receipt::from_json(&json.to_string()).unwrap();
        let Resource::Changed {
            slot: Slot::Json { file, key },
            edit,
            value,
            ..
        } = &read.resources[0]
        else {
            panic!("{:?}", read.resources)
        };
        assert!(file.ends_with(r"openvr\openvrpaths.vrpath"));
        assert_eq!(key, &["external_drivers"]);
        assert_eq!(*edit, Edit::Add);
        assert_eq!(value.as_str(), Some(r"C:\x\steamvr\aira"));
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
