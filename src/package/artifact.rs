use std::fmt;
use std::str::FromStr;

use super::Os;

/// one output kind. `setup` is the kago-owned installer, the rest are
/// native packages or bundles owned by their platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Artifact {
    Setup,
    Msi,
    App,
    Dmg,
    Pkg,
    AppImage,
    Deb,
    Rpm,
}

impl Artifact {
    pub const ALL: [Self; 8] = [
        Self::Setup,
        Self::Msi,
        Self::App,
        Self::Dmg,
        Self::Pkg,
        Self::AppImage,
        Self::Deb,
        Self::Rpm,
    ];

    /// name used in `targets.<os>.artifacts` and `when.format`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Msi => "msi",
            Self::App => "app",
            Self::Dmg => "dmg",
            Self::Pkg => "pkg",
            Self::AppImage => "appimage",
            Self::Deb => "deb",
            Self::Rpm => "rpm",
        }
    }

    #[must_use]
    pub const fn supports(self, os: Os) -> bool {
        matches!(
            (self, os),
            (Self::Setup, Os::Windows | Os::Linux)
                | (Self::Msi, Os::Windows)
                | (Self::App | Self::Dmg | Self::Pkg, Os::Macos)
                | (Self::AppImage | Self::Deb | Self::Rpm, Os::Linux)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownArtifact(pub String);

impl fmt::Display for UnknownArtifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown artifact `{}`, expected setup, msi, app, dmg, pkg, appimage, deb or rpm",
            self.0
        )
    }
}

impl std::error::Error for UnknownArtifact {}

impl FromStr for Artifact {
    type Err = UnknownArtifact;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|a| a.as_str() == s)
            .ok_or_else(|| UnknownArtifact(s.to_owned()))
    }
}

impl fmt::Display for Artifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for a in Artifact::ALL {
            assert_eq!(a.as_str().parse::<Artifact>(), Ok(a));
        }
        assert!("exe".parse::<Artifact>().is_err());
    }

    #[test]
    fn os_support() {
        assert!(Artifact::Setup.supports(Os::Windows));
        assert!(!Artifact::Setup.supports(Os::Macos));
        assert!(Artifact::Dmg.supports(Os::Macos));
        assert!(!Artifact::Msi.supports(Os::Linux));
    }
}
