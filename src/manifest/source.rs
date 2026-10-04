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
    None,
    Zip,
    TarGz,
}

impl FromStr for Unpack {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Self::None),
            "zip" => Ok(Self::Zip),
            "tar.gz" | "tgz" => Ok(Self::TarGz),
            _ => Err(format!(
                "unknown archive `{s}`, expected zip, tar.gz or none"
            )),
        }
    }
}

impl fmt::Display for Unpack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::None => "none",
            Self::Zip => "zip",
            Self::TarGz => "tar.gz",
        })
    }
}
