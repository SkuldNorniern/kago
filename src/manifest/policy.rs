//! `[install]` choices and the other closed keyword sets in the manifest.

/// a closed set of manifest keywords. `FromStr` errors name every allowed
/// value so the diagnostic can say what was meant.
macro_rules! keywords {
    ($(#[$doc:meta])* $ty:ident { $($variant:ident => $name:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $ty {
            $($variant,)+
        }

        impl $ty {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }
        }

        impl std::str::FromStr for $ty {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($name => Ok(Self::$variant),)+
                    _ => Err(format!(
                        "unknown value `{s}`, expected {}",
                        [$(concat!("`", $name, "`")),+].join(", ")
                    )),
                }
            }
        }

        impl std::fmt::Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        crate::string_serde!($ty);
    };
}

pub(crate) use keywords;

keywords! {
    /// `[install] scope`.
    ScopeChoice { User => "user", Machine => "machine", Choose => "choose", Auto => "auto" }
}

keywords! {
    /// `[install] scope_conflict`. what happens when the same package is
    /// already installed in the other scope.
    ScopeConflict { Exclusive => "exclusive", SideBySide => "allow-side-by-side" }
}

keywords! {
    /// `[install] elevation`.
    Elevation { Never => "never", WhenRequired => "when-required", Always => "always" }
}

keywords! {
    /// `[install] upgrade`.
    Upgrade { Replace => "replace" }
}

keywords! {
    /// `[install] downgrade`.
    Downgrade { Deny => "deny", Allow => "allow" }
}

keywords! {
    /// `[application] kind`.
    AppKind { Desktop => "desktop", Cli => "cli", Service => "service" }
}

keywords! {
    /// `[license] format`.
    LicenseFormat { Text => "text", Markdown => "markdown", Rtf => "rtf", Html => "html" }
}

/// `[install]` after defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPolicy {
    pub scope: ScopeChoice,
    pub scope_conflict: ScopeConflict,
    pub elevation: Elevation,
    pub upgrade: Upgrade,
    pub downgrade: Downgrade,
    pub repair: bool,
    pub rollback: bool,
}

impl Default for InstallPolicy {
    fn default() -> Self {
        Self {
            scope: ScopeChoice::User,
            scope_conflict: ScopeConflict::Exclusive,
            elevation: Elevation::WhenRequired,
            upgrade: Upgrade::Replace,
            downgrade: Downgrade::Deny,
            repair: true,
            rollback: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_errors_list_choices() {
        let e = "everyone".parse::<ScopeChoice>().unwrap_err();
        assert_eq!(
            e,
            "unknown value `everyone`, expected `user`, `machine`, `choose`, `auto`"
        );
        assert_eq!(
            "allow-side-by-side".parse::<ScopeConflict>(),
            Ok(ScopeConflict::SideBySide)
        );
    }
}
