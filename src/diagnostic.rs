//! diagnostics with stable `KAGO<nnn>` codes, source spans and help text.
//!
//! codes never change meaning. 1xx manifest, 2xx target and capability,
//! 3xx resolve.

use std::fmt;

use crate::package::Target;

/// stable diagnostic code, printed as `KAGO101`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Code(pub u16);

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KAGO{:03}", self.0)
    }
}

/// manifest is not valid TOML or does not match the schema.
pub const MANIFEST_SYNTAX: Code = Code(101);
/// manifest `schema` is missing or not supported by this kago.
pub const MANIFEST_SCHEMA: Code = Code(102);
/// a field value is invalid, like a bad package id or version.
pub const MANIFEST_VALUE: Code = Code(103);
/// two items share an id that must be unique.
pub const MANIFEST_DUPLICATE: Code = Code(104);
/// a reference to a component or logical resource that does not exist.
pub const MANIFEST_REFERENCE: Code = Code(105);
/// a semantic path uses an unknown root or escapes it.
pub const MANIFEST_PATH: Code = Code(106);
/// a `when` selector key or value is unknown.
pub const MANIFEST_SELECTOR: Code = Code(107);

/// target string is not a known target.
pub const TARGET_UNKNOWN: Code = Code(201);
/// target cannot provide what the manifest asks for.
pub const TARGET_CAPABILITY: Code = Code(204);

/// a required logical resource has no variant for the target.
pub const RESOLVE_MISSING: Code = Code(301);
/// more than one variant matches the target.
pub const RESOLVE_AMBIGUOUS: Code = Code(302);
/// two resources install to the same path.
pub const RESOLVE_CONFLICT: Code = Code(303);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Note,
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Note => "note",
            Self::Warning => "warning",
            Self::Error => "error",
        })
    }
}

/// `file:line:column`, 1-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

impl Location {
    /// line and column of byte `offset` in `source`. column counts chars.
    #[must_use]
    pub fn at(file: &str, source: &str, offset: usize) -> Self {
        let offset = offset.min(source.len());
        let before = source.get(..offset).unwrap_or(source);
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Self {
            file: file.to_owned(),
            line: count(before.matches('\n').count() + 1),
            column: count(before[line_start..].chars().count() + 1),
        }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: Code,
    pub severity: Severity,
    pub message: String,
    pub location: Option<Location>,
    pub target: Option<Target>,
    pub help: Option<String>,
}

impl Diagnostic {
    #[must_use]
    pub fn error(code: Code, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Error, message.into())
    }

    #[must_use]
    pub fn warning(code: Code, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Warning, message.into())
    }

    const fn new(code: Code, severity: Severity, message: String) -> Self {
        Self {
            code,
            severity,
            message,
            location: None,
            target: None,
            help: None,
        }
    }

    #[must_use]
    pub fn at(mut self, location: Location) -> Self {
        self.location = Some(location);
        self
    }

    #[must_use]
    pub fn for_target(mut self, target: Target) -> Self {
        self.target = Some(target);
        self
    }

    #[must_use]
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

/// ```text
/// error KAGO204
///  --> kago.toml:48:1
///
/// AppImage cannot install machine service `aurea-daemon`.
///
/// help:
///   use linux-setup, deb, or rpm
/// ```
impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} {}", self.severity, self.code)?;
        if let Some(location) = &self.location {
            writeln!(f, " --> {location}")?;
        }
        if let Some(target) = &self.target {
            writeln!(f, " target: {target}")?;
        }
        write!(f, "\n{}", self.message)?;
        if let Some(help) = &self.help {
            write!(f, "\n\nhelp:")?;
            for line in help.lines() {
                write!(f, "\n  {line}")?;
            }
        }
        Ok(())
    }
}

/// everything one pass found. errors and warnings together, in order found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl Diagnostics {
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.0.push(diagnostic);
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.0.iter().any(|d| d.severity == Severity::Error)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.0.iter()
    }
}

impl Extend<Diagnostic> for Diagnostics {
    fn extend<I: IntoIterator<Item = Diagnostic>>(&mut self, iter: I) {
        self.0.extend(iter);
    }
}

impl<'a> IntoIterator for &'a Diagnostics {
    type Item = &'a Diagnostic;
    type IntoIter = std::slice::Iter<'a, Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_zero_padded() {
        assert_eq!(Code(7).to_string(), "KAGO007");
        assert_eq!(TARGET_CAPABILITY.to_string(), "KAGO204");
    }

    #[test]
    fn location_from_offset() {
        let src = "schema = 1\n[package]\nid = \"x\"\n";
        let at = Location::at("kago.toml", src, src.find("id").unwrap());
        assert_eq!((at.line, at.column), (3, 1));
        assert_eq!(
            Location::at("kago.toml", src, 0).to_string(),
            "kago.toml:1:1"
        );
        assert_eq!(Location::at("k", "한글 x", "한글 ".len()).column, 4);
    }

    #[test]
    fn renders_human_form() {
        let d = Diagnostic::error(
            TARGET_CAPABILITY,
            "AppImage cannot install machine service `aurea-daemon`.",
        )
        .at(Location {
            file: "kago.toml".into(),
            line: 48,
            column: 1,
        })
        .help("use linux-setup, deb, or rpm");
        assert_eq!(
            d.to_string(),
            "error KAGO204\n --> kago.toml:48:1\n\nAppImage cannot install machine service `aurea-daemon`.\n\nhelp:\n  use linux-setup, deb, or rpm"
        );
    }

    #[test]
    fn has_errors_ignores_warnings() {
        let mut all = Diagnostics::default();
        all.push(Diagnostic::warning(MANIFEST_VALUE, "w"));
        assert!(!all.has_errors());
        all.push(Diagnostic::error(MANIFEST_VALUE, "e"));
        assert!(all.has_errors());
    }
}
