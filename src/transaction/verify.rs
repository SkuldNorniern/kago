use crate::install::hash;
use crate::manifest::Ownership;
use crate::receipt::{Receipt, Resource};

/// receipt against what is on disk now.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Verified {
    pub healthy: usize,
    pub missing: Vec<String>,
    /// owned files whose content changed.
    pub modified: Vec<String>,
    /// recorded but not checkable here, like PATH and registrations.
    pub unchecked: usize,
}

impl Verified {
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.missing.is_empty() && self.modified.is_empty()
    }
}

/// checks every file, directory and shortcut the receipt owns. files
/// users may change (config kept when modified, user data) only need to
/// exist.
#[must_use]
pub fn verify(receipt: &Receipt) -> Verified {
    let mut out = Verified::default();
    for resource in &receipt.resources {
        match resource {
            Resource::File {
                path,
                size,
                sha256,
                ownership,
            } => {
                let Some(abs) = receipt.absolute(path) else {
                    out.missing.push(path.to_string());
                    continue;
                };
                match hash(&abs) {
                    Err(_) => out.missing.push(abs.display().to_string()),
                    Ok(_) if !matches!(ownership, Ownership::Owned | Ownership::Generated) => {
                        out.healthy += 1;
                    }
                    Ok((s, d)) if s == *size && d == *sha256 => out.healthy += 1,
                    Ok(_) => out.modified.push(abs.display().to_string()),
                }
            }
            Resource::Directory { path } => match receipt.absolute(path) {
                Some(abs) if abs.is_dir() => out.healthy += 1,
                Some(abs) => out.missing.push(abs.display().to_string()),
                None => out.missing.push(path.to_string()),
            },
            Resource::Shortcut { path } => {
                if path.is_file() {
                    out.healthy += 1;
                } else {
                    out.missing.push(path.display().to_string());
                }
            }
            Resource::PathEntry { .. }
            | Resource::UninstallHook { .. }
            | Resource::LoginItem { .. }
            | Resource::FirewallRule { .. }
            | Resource::Service { .. }
            | Resource::Changed { .. }
            | Resource::Registration { .. } => out.unchecked += 1,
        }
    }
    out
}
