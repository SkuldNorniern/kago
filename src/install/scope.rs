use std::fmt;
use std::str::FromStr;

use super::Resolved;
use crate::diagnostic::{self, Diagnostic};
use crate::manifest::{InstallPolicy, ResourceScope, ScopeChoice};

/// where one install lands. `user` never needs elevation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    User,
    Machine,
}

impl Scope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Machine => "machine",
        }
    }

    /// scope a resource lands in for an install in `self`.
    #[must_use]
    pub const fn of(self, resource: ResourceScope) -> Self {
        match resource {
            ResourceScope::User => Self::User,
            ResourceScope::Machine => Self::Machine,
            ResourceScope::ResolvedInstall => self,
        }
    }
}

impl FromStr for Scope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Self::User),
            "machine" => Ok(Self::Machine),
            _ => Err(format!("unknown scope `{s}`, expected `user` or `machine`")),
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// final scope from the manifest policy and what the installer asked for.
/// `choose` takes the request or falls back to user; `auto` picks user
/// unless a resource has to be machine-wide.
///
/// # Errors
///
/// when the request contradicts a fixed manifest scope.
pub fn choose(
    policy: &InstallPolicy,
    resolved: &Resolved,
    requested: Option<Scope>,
) -> Result<Scope, Diagnostic> {
    let fixed = match policy.scope {
        ScopeChoice::User => Some(Scope::User),
        ScopeChoice::Machine => Some(Scope::Machine),
        ScopeChoice::Choose | ScopeChoice::Auto => None,
    };
    match (fixed, requested) {
        (Some(fixed), Some(requested)) if fixed != requested => Err(Diagnostic::error(
            diagnostic::INSTALL_SCOPE,
            format!("this package installs for {fixed} scope only, {requested} was asked for"),
        )
        .help(format!("drop `--scope` or use `--scope {fixed}`"))),
        (Some(fixed), _) => Ok(fixed),
        (None, Some(requested)) => Ok(requested),
        (None, None) if policy.scope == ScopeChoice::Auto && needs_machine(resolved) => {
            Ok(Scope::Machine)
        }
        (None, None) => Ok(Scope::User),
    }
}

fn needs_machine(resolved: &Resolved) -> bool {
    resolved
        .paths
        .iter()
        .any(|p| p.scope == ResourceScope::Machine)
        || resolved
            .services
            .iter()
            .any(|s| s.service.scope == ResourceScope::Machine)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::{Selection, resolve};
    use crate::manifest::{Context, Manifest, tests::AUREA};

    fn setup(scope: &str) -> (InstallPolicy, Resolved) {
        let src = AUREA.replace("scope = \"choose\"", &format!("scope = \"{scope}\""));
        let m = Manifest::parse("kago.toml", &src).0.unwrap();
        let cx = Context::target("windows-x86_64".parse().unwrap());
        let r = resolve(&m, &cx, Selection::defaults(&m)).0.unwrap();
        (m.install, r)
    }

    #[test]
    fn choose_takes_request_or_user() {
        let (p, r) = setup("choose");
        assert_eq!(choose(&p, &r, None), Ok(Scope::User));
        assert_eq!(choose(&p, &r, Some(Scope::Machine)), Ok(Scope::Machine));
    }

    #[test]
    fn fixed_scope_rejects_the_other() {
        let (p, r) = setup("user");
        assert_eq!(choose(&p, &r, Some(Scope::User)), Ok(Scope::User));
        let e = choose(&p, &r, Some(Scope::Machine)).unwrap_err();
        assert_eq!(e.code, diagnostic::INSTALL_SCOPE);
    }

    #[test]
    fn auto_stays_user_without_machine_resources() {
        let (p, r) = setup("auto");
        assert_eq!(choose(&p, &r, None), Ok(Scope::User));
    }
}
