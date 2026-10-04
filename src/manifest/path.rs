use std::fmt;
use std::str::FromStr;

/// install-side path like `{app}/bin/aurea.exe`. always a known root plus a
/// clean relative path, never an absolute OS path. the platform maps the
/// root per scope at install time.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticPath {
    pub root: Root,
    relative: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Root {
    App,
    Bin,
    Config,
    Data,
    Cache,
    Logs,
    Runtime,
    State,
    Desktop,
    Applications,
    UserHome,
    UserBin,
    SystemBin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// does not start with `{root}`.
    NoRoot,
    UnknownRoot(String),
    /// something other than `/` right after the root.
    NoSeparator,
    /// `..`, `.`, empty segment, `\`, `:` or a control character.
    BadSegment(String),
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRoot => f.write_str("path must start with a root like `{app}`"),
            Self::UnknownRoot(r) => write!(f, "unknown path root `{{{r}}}`"),
            Self::NoSeparator => f.write_str("expected `/` after the path root"),
            Self::BadSegment(s) => write!(
                f,
                "path segment `{s}` is not allowed; no `.`, `..`, empty segments, `\\`, `:` or control characters"
            ),
        }
    }
}

impl std::error::Error for PathError {}

impl Root {
    pub const ALL: [Self; 13] = [
        Self::App,
        Self::Bin,
        Self::Config,
        Self::Data,
        Self::Cache,
        Self::Logs,
        Self::Runtime,
        Self::State,
        Self::Desktop,
        Self::Applications,
        Self::UserHome,
        Self::UserBin,
        Self::SystemBin,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Bin => "bin",
            Self::Config => "config",
            Self::Data => "data",
            Self::Cache => "cache",
            Self::Logs => "logs",
            Self::Runtime => "runtime",
            Self::State => "state",
            Self::Desktop => "desktop",
            Self::Applications => "applications",
            Self::UserHome => "user-home",
            Self::UserBin => "user-bin",
            Self::SystemBin => "system-bin",
        }
    }
}

impl SemanticPath {
    /// segments after the root, joined by `/`. empty for the root itself.
    #[must_use]
    pub fn relative(&self) -> &str {
        &self.relative
    }

    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.relative.split('/').filter(|s| !s.is_empty())
    }

    /// directory holding this path. the root stays the root.
    #[must_use]
    pub fn parent(&self) -> Self {
        Self {
            root: self.root,
            relative: self
                .relative
                .rsplit_once('/')
                .map_or_else(String::new, |(parent, _)| parent.to_owned()),
        }
    }

    /// `segment` must already be one valid segment.
    #[must_use]
    pub fn join(&self, segment: &str) -> Self {
        let relative = if self.relative.is_empty() {
            segment.to_owned()
        } else {
            format!("{}/{segment}", self.relative)
        };
        Self {
            root: self.root,
            relative,
        }
    }
}

/// one path segment safe on every target filesystem.
pub(crate) fn valid_segment(s: &str) -> bool {
    !(s.is_empty()
        || s == "."
        || s == ".."
        || s.chars().any(|c| c == '\\' || c == ':' || c.is_control()))
}

impl FromStr for SemanticPath {
    type Err = PathError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s.strip_prefix('{').ok_or(PathError::NoRoot)?;
        let (name, rest) = rest.split_once('}').ok_or(PathError::NoRoot)?;
        let root = Root::ALL
            .into_iter()
            .find(|r| r.as_str() == name)
            .ok_or_else(|| PathError::UnknownRoot(name.to_owned()))?;
        let relative = match rest {
            "" | "/" => "",
            _ => rest.strip_prefix('/').ok_or(PathError::NoSeparator)?,
        };
        if !relative.is_empty()
            && let Some(bad) = relative.split('/').find(|s| !valid_segment(s))
        {
            return Err(PathError::BadSegment(bad.to_owned()));
        }
        Ok(Self {
            root,
            relative: relative.to_owned(),
        })
    }
}

impl FromStr for Root {
    type Err = PathError;

    /// bare name like `app`, as used in receipts.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|r| r.as_str() == s)
            .ok_or_else(|| PathError::UnknownRoot(s.to_owned()))
    }
}

impl serde::Serialize for Root {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for Root {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <String as serde::Deserialize>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for Root {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{}}}", self.as_str())
    }
}

impl fmt::Display for SemanticPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.relative.is_empty() {
            write!(f, "{}", self.root)
        } else {
            write!(f, "{}/{}", self.root, self.relative)
        }
    }
}

crate::string_serde!(SemanticPath);

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> SemanticPath {
        s.parse().unwrap()
    }

    #[test]
    fn parses_and_prints() {
        let path = p("{app}/bin/aurea.exe");
        assert_eq!(path.root, Root::App);
        assert_eq!(path.relative(), "bin/aurea.exe");
        assert_eq!(path.to_string(), "{app}/bin/aurea.exe");
        assert_eq!(p("{user-bin}").relative(), "");
        assert_eq!(p("{config}/").to_string(), "{config}");
    }

    #[test]
    fn parent_and_join() {
        let path = p("{app}/bin/aurea.exe");
        assert_eq!(path.parent(), p("{app}/bin"));
        assert_eq!(path.parent().parent(), p("{app}"));
        assert_eq!(p("{app}").join("bin"), p("{app}/bin"));
    }

    #[test]
    fn rejects_escapes() {
        let err = |s: &str| s.parse::<SemanticPath>().unwrap_err();
        assert_eq!(err("C:/Program Files/x"), PathError::NoRoot);
        assert_eq!(err("/usr/bin/x"), PathError::NoRoot);
        assert_eq!(err("{windows}/x"), PathError::UnknownRoot("windows".into()));
        assert_eq!(err("{app}bin"), PathError::NoSeparator);
        assert_eq!(err("{app}/../x"), PathError::BadSegment("..".into()));
        assert_eq!(err("{app}/a//b"), PathError::BadSegment(String::new()));
        assert_eq!(err("{app}/a\\b"), PathError::BadSegment("a\\b".into()));
        assert_eq!(err("{app}/c:x"), PathError::BadSegment("c:x".into()));
        assert_eq!(err("{app}/a/./b"), PathError::BadSegment(".".into()));
    }
}
