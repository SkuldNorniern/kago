//! what an install needs on the machine before it starts: an OS version
//! or software like SteamVR. checked before anything changes.

use std::fmt;

use crate::diagnostic::{self, Diagnostic};
use crate::manifest::{Detect, Requirement, RequirementKind, RequirementPolicy};
use crate::package::Os;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "need", rename_all = "kebab-case")]
pub enum Need {
    /// this OS at `minimum` or newer, dotted numbers like `10.0.19041`.
    Os { os: Os, minimum: String },
    /// software found on the machine.
    Software { detect: Detect },
}

impl fmt::Display for Need {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Os { os, minimum } => write!(f, "{} {minimum} or newer", os_name(*os)),
            Self::Software { detect } => f.write_str(detect.name()),
        }
    }
}

const fn os_name(os: Os) -> &'static str {
    match os {
        Os::Windows => "Windows",
        Os::Macos => "macOS",
        Os::Linux => "Linux",
    }
}

impl Need {
    /// the need a manifest requirement stands for.
    ///
    /// # Errors
    ///
    /// when kago cannot check or meet it yet.
    pub fn from_requirement(r: &Requirement) -> Result<Self, Diagnostic> {
        let unsupported = |what: String| Diagnostic::error(diagnostic::TARGET_CAPABILITY, what);
        if r.policy == RequirementPolicy::InstallIfMissing {
            return Err(unsupported(format!(
                "`require {} install-if-missing` is not supported yet; kago can only check for it",
                r.id
            )));
        }
        match r.kind {
            RequirementKind::Os => {
                let os = r
                    .id
                    .parse::<Os>()
                    .map_err(|e| Diagnostic::error(diagnostic::MANIFEST_VALUE, e.to_string()))?;
                let minimum = r.minimum.clone().ok_or_else(|| {
                    Diagnostic::error(
                        diagnostic::MANIFEST_VALUE,
                        format!("`require {}` needs a minimum version", r.id),
                    )
                })?;
                if !minimum.split('.').all(|p| p.parse::<u64>().is_ok()) {
                    return Err(Diagnostic::error(
                        diagnostic::MANIFEST_VALUE,
                        format!("`{minimum}` is not a dotted version like 10.0.19041"),
                    ));
                }
                Ok(Self::Os { os, minimum })
            }
            RequirementKind::Runtime => {
                r.id.parse::<Detect>()
                    .map(|detect| Self::Software { detect })
                    .map_err(|e| unsupported(format!("cannot check for `{}` yet: {e}", r.id)))
            }
            RequirementKind::SystemPackage => Err(unsupported(format!(
                "system package `{}` cannot be checked yet",
                r.id
            ))),
        }
    }
}

/// whether dotted version `have` is at least `need`; missing parts count
/// as 0.
#[must_use]
pub fn at_least(have: &str, need: &str) -> bool {
    let parts = |s: &str| -> Vec<u64> { s.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    let (have, need) = (parts(have), parts(need));
    for i in 0..have.len().max(need.len()) {
        let (a, b) = (
            have.get(i).copied().unwrap_or(0),
            need.get(i).copied().unwrap_or(0),
        );
        if a != b {
            return a > b;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(at_least("10.0.22631", "10.0.19041"));
        assert!(at_least("10.0.19041", "10.0.19041"));
        assert!(!at_least("10.0.17763", "10.0.19041"));
        assert!(at_least("11", "10.0.19041"));
        assert!(at_least("10.0", "10"));
    }

    #[test]
    fn needs_read_as_sentences() {
        let os = Need::Os {
            os: Os::Windows,
            minimum: "10.0.19041".into(),
        };
        assert_eq!(os.to_string(), "Windows 10.0.19041 or newer");
        let vr = Need::Software {
            detect: Detect::SteamVr,
        };
        assert_eq!(vr.to_string(), "SteamVR");
    }
}
