//! where payload comes from: a local directory or a pinned download.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use crate::diagnostic::Location;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub kind: SourceKind,
    /// expected sha-256 of the download, per target (`None` for any).
    pub sha256: BTreeMap<Option<String>, String>,
    pub unpack: Unpack,
    pub at: Location,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    /// relative to the Kagofile, `${...}` allowed.
    Dir(String),
    /// URL, `${...}` allowed.
    Fetch(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unpack {
    /// from the URL: `.zip`, `.tar.gz` / `.tgz`, anything else as is.
    Auto,
    None,
    Zip,
    TarGz,
}

impl FromStr for Unpack {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(Self::Auto),
            "none" => Ok(Self::None),
            "zip" => Ok(Self::Zip),
            "tar.gz" | "tgz" => Ok(Self::TarGz),
            _ => Err(format!(
                "unknown archive `{s}`, expected auto, zip, tar.gz or none"
            )),
        }
    }
}

impl fmt::Display for Unpack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::None => "none",
            Self::Zip => "zip",
            Self::TarGz => "tar.gz",
        })
    }
}

impl Unpack {
    /// `Auto` decided by the file name of `url`.
    #[allow(clippy::case_sensitive_file_extension_comparisons, reason = "lowercased first")]
    #[must_use]
    pub fn for_url(self, url: &str) -> Self {
        if self != Self::Auto {
            return self;
        }
        let name = url.rsplit('/').next().unwrap_or(url).to_ascii_lowercase();
        if name.ends_with(".zip") {
            Self::Zip
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Self::TarGz
        } else {
            Self::None
        }
    }
}
