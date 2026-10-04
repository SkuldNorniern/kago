use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

/// canonical package version, SemVer 2.0. backends map it to MSI, deb, rpm
/// and bundle versions, never the other way.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// `beta.2` in `1.4.0-beta.2`. empty for a release.
    pub pre: Vec<Identifier>,
    /// `build.7` in `1.4.0+build.7`. no effect on precedence.
    pub build: Vec<String>,
}

/// one dot-separated pre-release part.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Identifier {
    Numeric(u64),
    Alpha(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionError {
    /// not `major.minor.patch`.
    Shape,
    /// a numeric part is empty, not a number, too large or has a leading zero.
    Number(String),
    EmptyIdentifier,
    /// only `0-9`, `A-Z`, `a-z` and `-` are allowed in identifiers.
    BadChar(char),
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shape => f.write_str("version is not `major.minor.patch`"),
            Self::Number(n) => write!(f, "version part `{n}` is not a valid number"),
            Self::EmptyIdentifier => f.write_str("version has an empty pre-release or build part"),
            Self::BadChar(c) => write!(
                f,
                "version has `{c}`, only 0-9, A-Z, a-z and `-` are allowed"
            ),
        }
    }
}

impl std::error::Error for VersionError {}

impl Version {
    #[must_use]
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
            pre: Vec::new(),
            build: Vec::new(),
        }
    }

    /// # Errors
    ///
    /// when `s` is not a SemVer 2.0 version.
    pub fn parse(s: &str) -> Result<Self, VersionError> {
        let (rest, build) = match s.split_once('+') {
            Some((rest, build)) => (rest, parts(build)?),
            None => (s, Vec::new()),
        };
        let (core, pre) = match rest.split_once('-') {
            Some((core, pre)) => (core, pre_identifiers(pre)?),
            None => (rest, Vec::new()),
        };
        let mut numbers = core.split('.');
        let (Some(major), Some(minor), Some(patch), None) = (
            numbers.next(),
            numbers.next(),
            numbers.next(),
            numbers.next(),
        ) else {
            return Err(VersionError::Shape);
        };
        Ok(Self {
            major: number(major)?,
            minor: number(minor)?,
            patch: number(patch)?,
            pre,
            build: build.into_iter().map(str::to_owned).collect(),
        })
    }

    #[must_use]
    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }

    /// SemVer precedence. build metadata is ignored, so `1.0.0+a` and
    /// `1.0.0+b` are equal here while `Ord` still tells them apart.
    #[must_use]
    pub fn cmp_precedence(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            })
    }
}

impl Ord for Version {
    /// precedence first, then build metadata so the order stays total.
    fn cmp(&self, other: &Self) -> Ordering {
        self.cmp_precedence(other)
            .then_with(|| self.build.cmp(&other.build))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Identifier {
    /// numeric below alphanumeric, numbers by value, text by ASCII.
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Numeric(a), Self::Numeric(b)) => a.cmp(b),
            (Self::Numeric(_), Self::Alpha(_)) => Ordering::Less,
            (Self::Alpha(_), Self::Numeric(_)) => Ordering::Greater,
            (Self::Alpha(a), Self::Alpha(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for Identifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn number(s: &str) -> Result<u64, VersionError> {
    let bad = || VersionError::Number(s.to_owned());
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0'))
    {
        return Err(bad());
    }
    s.parse().map_err(|_| bad())
}

fn parts(s: &str) -> Result<Vec<&str>, VersionError> {
    s.split('.')
        .map(|part| {
            if part.is_empty() {
                return Err(VersionError::EmptyIdentifier);
            }
            match part
                .chars()
                .find(|c| !(c.is_ascii_alphanumeric() || *c == '-'))
            {
                Some(c) => Err(VersionError::BadChar(c)),
                None => Ok(part),
            }
        })
        .collect()
}

fn pre_identifiers(s: &str) -> Result<Vec<Identifier>, VersionError> {
    parts(s)?
        .into_iter()
        .map(|part| {
            if part.bytes().all(|b| b.is_ascii_digit()) {
                number(part).map(Identifier::Numeric)
            } else {
                Ok(Identifier::Alpha(part.to_owned()))
            }
        })
        .collect()
}

impl FromStr for Version {
    type Err = VersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Numeric(n) => write!(f, "{n}"),
            Self::Alpha(s) => f.write_str(s),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        for (i, id) in self.pre.iter().enumerate() {
            write!(f, "{}{id}", if i == 0 { '-' } else { '.' })?;
        }
        for (i, part) in self.build.iter().enumerate() {
            write!(f, "{}{part}", if i == 0 { '+' } else { '.' })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn round_trips() {
        for s in [
            "1.4.0",
            "1.4.0-beta.2",
            "0.0.1-alpha-1.x.7+build.11.e0f985a",
            "10.20.30+meta",
        ] {
            assert_eq!(v(s).to_string(), s);
        }
    }

    #[test]
    fn rejects_invalid() {
        assert_eq!(Version::parse("1.4"), Err(VersionError::Shape));
        assert_eq!(Version::parse("1.4.0.1"), Err(VersionError::Shape));
        assert_eq!(
            Version::parse("01.4.0"),
            Err(VersionError::Number("01".into()))
        );
        assert_eq!(
            Version::parse("1.4.0-beta.02"),
            Err(VersionError::Number("02".into()))
        );
        assert_eq!(Version::parse("1.4.0-"), Err(VersionError::EmptyIdentifier));
        assert_eq!(
            Version::parse("1.4.0-beta..1"),
            Err(VersionError::EmptyIdentifier)
        );
        assert_eq!(
            Version::parse("1.4.0-beta_1"),
            Err(VersionError::BadChar('_'))
        );
        assert_eq!(
            Version::parse("v1.4.0"),
            Err(VersionError::Number("v1".into()))
        );
    }

    #[test]
    fn build_allows_leading_zero() {
        assert_eq!(v("1.0.0+001").build, vec!["001"]);
    }

    #[test]
    fn semver_spec_order() {
        let ordered = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1.0",
            "2.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn build_ignored_for_precedence() {
        assert_eq!(v("1.0.0+a").cmp_precedence(&v("1.0.0+b")), Ordering::Equal);
        assert_ne!(v("1.0.0+a"), v("1.0.0+b"));
    }
}
