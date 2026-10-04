use std::str::FromStr;

use serde_spanned::Spanned;

use super::schema::RawWhen;
use crate::diagnostic::{self, Diagnostic, Diagnostics, Location};
use crate::package::{Arch, Artifact, Libc, Os, Target};

/// a `when` predicate. every key that is set must match, values inside one
/// key are alternatives. no keys means always.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selector {
    pub os: Option<Vec<Os>>,
    pub arch: Option<Vec<Arch>>,
    pub libc: Option<Vec<Libc>>,
    pub format: Option<Vec<Artifact>>,
    pub profile: Option<Vec<String>>,
    pub channel: Option<Vec<String>>,
}

/// what a selector is checked against. a selector key with no value here
/// does not match, so `when.format` never matches when no format is chosen.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    pub target: Target,
    pub format: Option<Artifact>,
    pub profile: Option<&'a str>,
    pub channel: Option<&'a str>,
}

impl Context<'_> {
    #[must_use]
    pub const fn target(target: Target) -> Self {
        Self {
            target,
            format: None,
            profile: None,
            channel: None,
        }
    }
}

impl Selector {
    #[must_use]
    pub fn matches(&self, cx: &Context<'_>) -> bool {
        fn has<T: PartialEq>(key: Option<&Vec<T>>, value: Option<&T>) -> bool {
            key.is_none_or(|values| value.is_some_and(|v| values.contains(v)))
        }
        has(self.os.as_ref(), Some(&cx.target.os()))
            && has(self.arch.as_ref(), Some(&cx.target.arch()))
            && has(self.libc.as_ref(), cx.target.libc().as_ref())
            && has(self.format.as_ref(), cx.format.as_ref())
            && has(
                self.profile.as_ref(),
                cx.profile.map(str::to_owned).as_ref(),
            )
            && has(
                self.channel.as_ref(),
                cx.channel.map(str::to_owned).as_ref(),
            )
    }

    #[must_use]
    pub fn is_always(&self) -> bool {
        *self == Self::default()
    }
}

/// raw `when` to a checked selector. bad values become diagnostics and are
/// left out, so one typo gives one error, not a cascade.
pub(super) fn selector(
    raw: Option<&RawWhen>,
    file: &str,
    source: &str,
    diagnostics: &mut Diagnostics,
) -> Selector {
    let Some(raw) = raw else {
        return Selector::default();
    };
    let mut cx = Values {
        file,
        source,
        diagnostics,
    };
    Selector {
        os: cx.parse("os", raw.os.as_ref()),
        arch: cx.parse("arch", raw.arch.as_ref()),
        libc: cx.parse("libc", raw.libc.as_ref()),
        format: cx.parse("format", raw.format.as_ref()),
        profile: cx.parse("profile", raw.profile.as_ref()),
        channel: cx.parse("channel", raw.channel.as_ref()),
    }
}

struct Values<'a> {
    file: &'a str,
    source: &'a str,
    diagnostics: &'a mut Diagnostics,
}

impl Values<'_> {
    fn parse<T>(&mut self, key: &str, values: Option<&Spanned<Vec<String>>>) -> Option<Vec<T>>
    where
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        let values = values?;
        let at = Location::at(self.file, self.source, values.span().start);
        let mut out = Vec::new();
        for value in values.get_ref() {
            let error = if value.is_empty() {
                Some("empty value".to_owned())
            } else {
                match value.parse::<T>() {
                    Ok(parsed) => {
                        out.push(parsed);
                        None
                    }
                    Err(e) => Some(e.to_string()),
                }
            };
            if let Some(error) = error {
                self.diagnostics.push(
                    Diagnostic::error(
                        diagnostic::MANIFEST_SELECTOR,
                        format!("`when.{key}`: {error}"),
                    )
                    .at(at.clone()),
                );
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// raw `when` from `key=a,b` pairs.
    fn when(pairs: &[(&str, &str)]) -> (Selector, Diagnostics) {
        let mut raw = RawWhen::default();
        for (key, values) in pairs {
            let list = Some(Spanned::new(
                0..1,
                values.split(',').map(str::to_owned).collect::<Vec<_>>(),
            ));
            match *key {
                "os" => raw.os = list,
                "arch" => raw.arch = list,
                "libc" => raw.libc = list,
                "format" => raw.format = list,
                "profile" => raw.profile = list,
                _ => panic!("{key}"),
            }
        }
        let mut diagnostics = Diagnostics::default();
        let s = selector(Some(&raw), "Kagofile", "x", &mut diagnostics);
        (s, diagnostics)
    }

    fn cx(target: &str) -> Context<'static> {
        Context::target(target.parse().unwrap())
    }

    #[test]
    fn matches_keys_and_alternatives() {
        let (s, d) = when(&[("os", "windows"), ("arch", "x86_64,aarch64")]);
        assert!(d.is_empty());
        assert!(s.matches(&cx("windows-x86_64")));
        assert!(s.matches(&cx("windows-aarch64")));
        assert!(!s.matches(&cx("linux-x86_64")));
    }

    #[test]
    fn libc_needs_a_libc() {
        let (s, _) = when(&[("libc", "musl")]);
        assert!(s.matches(&cx("linux-x86_64-musl")));
        assert!(!s.matches(&cx("linux-x86_64-gnu")));
        assert!(!s.matches(&cx("windows-x86_64")));
    }

    #[test]
    fn format_needs_a_chosen_format() {
        let (s, _) = when(&[("format", "msi")]);
        let mut c = cx("windows-x86_64");
        assert!(!s.matches(&c));
        c.format = Some(Artifact::Msi);
        assert!(s.matches(&c));
    }

    #[test]
    fn empty_selector_always_matches() {
        assert!(Selector::default().is_always());
        assert!(Selector::default().matches(&cx("macos-aarch64")));
    }

    #[test]
    fn bad_values_are_reported_once_each() {
        let (s, d) = when(&[("os", "windows,freebsd"), ("profile", "")]);
        assert_eq!(s.os, Some(vec![Os::Windows]));
        assert_eq!(d.iter().count(), 2);
        assert!(d.iter().all(|d| d.code == diagnostic::MANIFEST_SELECTOR));
    }
}
