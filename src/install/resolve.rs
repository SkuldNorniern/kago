use std::collections::{BTreeMap, BTreeSet};

use crate::diagnostic::{self, Diagnostic, Diagnostics, Location};
use crate::manifest::{
    Change, Context, File, Hook, Manifest, PathKind, PathSource, Requirement, ResourceScope,
    Selector, SemanticPath, Service, Shortcut, SteamVrDriver,
};

/// components picked for one install. items without a component are
/// always in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection(BTreeSet<String>);

impl Selection {
    /// required and default components, plus everything they depend on.
    #[must_use]
    pub fn defaults(manifest: &Manifest) -> Self {
        Self::with(
            manifest,
            manifest
                .components
                .iter()
                .filter(|c| c.default)
                .map(|c| c.id.as_str()),
        )
    }

    /// `picked` plus required components plus dependencies. unknown ids are
    /// ignored here, the caller checks them against the manifest.
    #[must_use]
    pub fn with<'a>(manifest: &Manifest, picked: impl IntoIterator<Item = &'a str>) -> Self {
        let mut todo: Vec<String> = manifest
            .components
            .iter()
            .filter(|c| c.required)
            .map(|c| c.id.clone())
            .chain(picked.into_iter().map(str::to_owned))
            .collect();
        let mut set = BTreeSet::new();
        while let Some(id) = todo.pop() {
            if let Some(c) = manifest.component(&id)
                && set.insert(id)
            {
                todo.extend(c.depends.iter().cloned());
            }
        }
        Self(set)
    }

    #[must_use]
    pub fn contains(&self, component: Option<&str>) -> bool {
        component.is_none_or(|c| self.0.contains(c))
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}

/// the manifest narrowed to one target and component selection. every
/// logical id points at exactly one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub files: Vec<File>,
    pub paths: Vec<ResolvedPath>,
    pub steamvr_drivers: Vec<SteamVrDriver>,
    /// requirements for this target.
    pub requirements: Vec<Requirement>,
    pub shortcuts: Vec<ResolvedShortcut>,
    pub services: Vec<ResolvedService>,
    pub hooks: Vec<ResolvedHook>,
    /// values set outside the package, for this target.
    pub changes: Vec<Change>,
    pub components: Selection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    pub directory: SemanticPath,
    pub scope: ResourceScope,
    pub component: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedShortcut {
    pub shortcut: Shortcut,
    pub target: SemanticPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedService {
    pub service: Service,
    pub command: SemanticPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHook {
    pub hook: Hook,
    pub command: SemanticPath,
}

/// picks the variant of every resource for `cx`. missing or ambiguous
/// variants are errors on the item that needs them.
#[must_use]
pub fn resolve(
    manifest: &Manifest,
    cx: &Context<'_>,
    components: Selection,
) -> (Option<Resolved>, Diagnostics) {
    let files: Vec<File> = manifest
        .files
        .iter()
        .filter(|f| components.contains(f.component.as_deref()) && f.when.matches(cx))
        .cloned()
        .collect();
    let mut r = Resolver {
        cx,
        components: &components,
        variants: BTreeMap::new(),
        diagnostics: Diagnostics::default(),
    };
    for f in &files {
        if let Some(id) = &f.logical_id {
            r.variants.entry(id.clone()).or_default().push(f.clone());
        }
    }
    r.check_variants();
    r.check_conflicts(&files);
    let paths = r.paths(manifest);
    let steamvr_drivers = manifest
        .steamvr_drivers
        .iter()
        .filter(|d| r.active(d.component.as_deref(), &d.when))
        .cloned()
        .collect();
    let requirements = manifest
        .requirements
        .iter()
        .filter(|q| q.when.matches(cx))
        .cloned()
        .collect();
    let shortcuts = r.shortcuts(manifest);
    let services = r.services(manifest);
    let hooks = r.hooks(manifest);
    let changes = manifest
        .changes
        .iter()
        .filter(|c| r.active(c.component.as_deref(), &c.when))
        .cloned()
        .collect();
    if let Some(entry) = &manifest.application.entrypoint {
        r.lookup("`application.entrypoint`", entry, None);
    }
    let diagnostics = r.diagnostics;
    let resolved = Resolved {
        files,
        paths,
        steamvr_drivers,
        requirements,
        shortcuts,
        services,
        hooks,
        changes,
        components,
    };
    ((!diagnostics.has_errors()).then_some(resolved), diagnostics)
}

struct Resolver<'a> {
    cx: &'a Context<'a>,
    components: &'a Selection,
    /// active files per logical id.
    variants: BTreeMap<String, Vec<File>>,
    diagnostics: Diagnostics,
}

impl Resolver<'_> {
    fn active(&self, component: Option<&str>, when: &Selector) -> bool {
        self.components.contains(component) && when.matches(self.cx)
    }

    fn check_variants(&mut self) {
        for (id, variants) in &self.variants {
            if let [first, second, ..] = variants.as_slice() {
                self.diagnostics.push(
                    Diagnostic::error(
                        diagnostic::RESOLVE_AMBIGUOUS,
                        format!(
                            "`{id}` has {} variants for this target, at lines {} and {}",
                            variants.len(),
                            first.at.line,
                            second.at.line
                        ),
                    )
                    .at(second.at.clone())
                    .for_target(self.cx.target)
                    .help("narrow the `when` selectors so exactly one variant matches"),
                );
            }
        }
    }

    fn check_conflicts(&mut self, files: &[File]) {
        let mut targets: BTreeMap<&SemanticPath, &File> = BTreeMap::new();
        for f in files {
            if let Some(other) = targets.insert(&f.target, f) {
                self.diagnostics.push(
                    Diagnostic::error(
                        diagnostic::RESOLVE_CONFLICT,
                        format!(
                            "`{}` is installed by two files, the other at line {}",
                            f.target, other.at.line
                        ),
                    )
                    .at(f.at.clone())
                    .for_target(self.cx.target),
                );
            }
        }
    }

    /// target of the one variant of `id`, or a missing-variant error.
    fn lookup(&mut self, what: &str, id: &str, at: Option<&Location>) -> Option<SemanticPath> {
        if let Some([one, ..]) = self.variants.get(id).map(Vec::as_slice) {
            return Some(one.target.clone());
        }
        let mut d = Diagnostic::error(
            diagnostic::RESOLVE_MISSING,
            format!("{what} needs `{id}`, which has no variant for this target"),
        )
        .for_target(self.cx.target)
        .help(format!(
            "add a `[[files]]` entry with `logical_id = \"{id}\"` matching {}, or limit this item with `when`",
            self.cx.target
        ));
        if let Some(at) = at {
            d = d.at(at.clone());
        }
        self.diagnostics.push(d);
        None
    }

    fn paths(&mut self, manifest: &Manifest) -> Vec<ResolvedPath> {
        let mut out = Vec::new();
        for p in &manifest.paths {
            if !self.active(p.component.as_deref(), &p.when) {
                continue;
            }
            let directory = match &p.source {
                PathSource::Directory(dir) => Some(dir.clone()),
                PathSource::Resource {
                    id,
                    kind: PathKind::ExecutableParent,
                } => self
                    .lookup("`[[paths]]`", id, Some(&p.at))
                    .map(|t| t.parent()),
            };
            if let Some(directory) = directory {
                out.push(ResolvedPath {
                    directory,
                    scope: p.scope,
                    component: p.component.clone(),
                });
            }
        }
        out
    }

    fn shortcuts(&mut self, manifest: &Manifest) -> Vec<ResolvedShortcut> {
        let mut out = Vec::new();
        for s in &manifest.shortcuts {
            if self.active(s.component.as_deref(), &s.when)
                && let Some(target) = self.lookup("shortcut", &s.target, Some(&s.at))
            {
                out.push(ResolvedShortcut {
                    shortcut: s.clone(),
                    target,
                });
            }
        }
        out
    }

    fn services(&mut self, manifest: &Manifest) -> Vec<ResolvedService> {
        let mut out = Vec::new();
        for s in &manifest.services {
            if self.active(s.component.as_deref(), &s.when)
                && let Some(command) = self.lookup("service", &s.command, Some(&s.at))
            {
                out.push(ResolvedService {
                    service: s.clone(),
                    command,
                });
            }
        }
        out
    }

    fn hooks(&mut self, manifest: &Manifest) -> Vec<ResolvedHook> {
        let mut out = Vec::new();
        for h in &manifest.hooks {
            if h.when.matches(self.cx)
                && let Some(command) = self.lookup("hook", &h.command, Some(&h.at))
            {
                out.push(ResolvedHook {
                    hook: h.clone(),
                    command,
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::AUREA;

    fn manifest(src: &str) -> Manifest {
        let (m, d) = Manifest::parse("Kagofile", src);
        m.unwrap_or_else(|| panic!("{d:?}"))
    }

    fn resolve_for(m: &Manifest, target: &str) -> (Option<Resolved>, Diagnostics) {
        resolve(
            m,
            &Context::target(target.parse().unwrap()),
            Selection::defaults(m),
        )
    }

    #[test]
    fn picks_one_variant_per_target() {
        let m = manifest(AUREA);
        let (r, d) = resolve_for(&m, "windows-x86_64");
        assert!(d.is_empty(), "{d:?}");
        let r = r.unwrap();
        let targets: Vec<String> = r.files.iter().map(|f| f.target.to_string()).collect();
        assert_eq!(targets, ["{app}/bin/aurea.exe", "{app}/themes"]);
        assert_eq!(r.shortcuts[0].target.to_string(), "{app}/bin/aurea.exe");
        assert_eq!(r.paths[0].directory.to_string(), "{app}/bin");

        let (r, _) = resolve_for(&m, "macos-aarch64");
        let r = r.unwrap();
        assert_eq!(
            r.shortcuts[0].target.to_string(),
            "{app}/Contents/MacOS/Aurea"
        );
        assert!(r.paths.is_empty(), "no PATH on macos");
    }

    #[test]
    fn missing_variant_is_an_error() {
        let m = manifest(AUREA);
        let (r, d) = resolve_for(&m, "windows-aarch64");
        assert!(r.is_none());
        let codes: BTreeSet<_> = d.iter().map(|d| d.code).collect();
        assert_eq!(codes, BTreeSet::from([diagnostic::RESOLVE_MISSING]));
    }

    #[test]
    fn ambiguous_variant_is_an_error() {
        let src = format!("{AUREA}\napp main-executable\n  windows other.exe\n");
        let m = manifest(&src);
        let (_, d) = resolve_for(&m, "windows-x86_64");
        assert!(d.iter().any(|d| d.code == diagnostic::RESOLVE_AMBIGUOUS));
    }

    #[test]
    fn same_target_twice_is_a_conflict() {
        let src = format!("{AUREA}\ncopy x -> {{app}}/themes\n");
        let m = manifest(&src);
        let (_, d) = resolve_for(&m, "linux-x86_64");
        assert!(d.iter().any(|d| d.code == diagnostic::RESOLVE_CONFLICT));
    }

    #[test]
    fn components_gate_items() {
        let m = manifest(AUREA);
        let only_core = Selection::with(&m, []);
        assert!(only_core.contains(Some("core")));
        assert!(!only_core.contains(Some("cli")));
        let (r, _) = resolve(
            &m,
            &Context::target("linux-x86_64".parse().unwrap()),
            only_core,
        );
        assert!(r.unwrap().paths.is_empty(), "PATH belongs to cli");
        assert!(Selection::with(&m, ["cli"]).contains(Some("core")));
    }
}
