use std::fmt;
use std::str::FromStr;

/// stable package id, reverse-DNS like `dev.eira.aurea`. never changes
/// across versions, display names can.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdError {
    Empty,
    TooLong,
    /// fewer than two dot-separated segments.
    NotQualified,
    EmptySegment,
    /// only `a-z`, `0-9` and `-` are allowed.
    BadChar(char),
    /// segment starts or ends with `-`.
    EdgeHyphen,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("package id is empty"),
            Self::TooLong => write!(f, "package id is over {MAX_LEN} bytes"),
            Self::NotQualified => {
                f.write_str("package id needs at least two segments, like `dev.eira.aurea`")
            }
            Self::EmptySegment => f.write_str("package id has an empty segment"),
            Self::BadChar(c) => write!(
                f,
                "package id has `{c}`, only a-z, 0-9, `-` and `.` are allowed"
            ),
            Self::EdgeHyphen => f.write_str("package id segment starts or ends with `-`"),
        }
    }
}

impl std::error::Error for IdError {}

const MAX_LEN: usize = 255;

impl PackageId {
    /// # Errors
    ///
    /// when `id` is not a lowercase reverse-DNS id.
    pub fn new(id: &str) -> Result<Self, IdError> {
        if id.is_empty() {
            return Err(IdError::Empty);
        }
        if id.len() > MAX_LEN {
            return Err(IdError::TooLong);
        }
        let mut segments = 0;
        for segment in id.split('.') {
            segments += 1;
            if segment.is_empty() {
                return Err(IdError::EmptySegment);
            }
            if let Some(c) = segment
                .chars()
                .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
            {
                return Err(IdError::BadChar(c));
            }
            if segment.starts_with('-') || segment.ends_with('-') {
                return Err(IdError::EdgeHyphen);
            }
        }
        if segments < 2 {
            return Err(IdError::NotQualified);
        }
        Ok(Self(id.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for PackageId {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

crate::string_serde!(PackageId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_reverse_dns() {
        assert_eq!(
            PackageId::new("dev.eira.aurea").unwrap().as_str(),
            "dev.eira.aurea"
        );
        assert!(PackageId::new("com.example.my-app2").is_ok());
    }

    #[test]
    fn rejects_bad_ids() {
        assert_eq!(PackageId::new(""), Err(IdError::Empty));
        assert_eq!(PackageId::new("aurea"), Err(IdError::NotQualified));
        assert_eq!(PackageId::new("dev..aurea"), Err(IdError::EmptySegment));
        assert_eq!(PackageId::new("dev.Eira.aurea"), Err(IdError::BadChar('E')));
        assert_eq!(PackageId::new("dev.eira/aurea"), Err(IdError::BadChar('/')));
        assert_eq!(PackageId::new("dev.-eira.aurea"), Err(IdError::EdgeHyphen));
        assert_eq!(PackageId::new(&"a.".repeat(200)), Err(IdError::TooLong));
    }
}
