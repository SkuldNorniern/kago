use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use serde_spanned::Spanned;

use super::path::SemanticPath;
use super::policy::{AppKind, Detect, InstallPolicy, LicenseFormat};
use super::resource::{
    Component, File, Hook, HookPrivilege, Icon, Ownership, PathEntry, PathKind, PathSource,
    Requirement, RequirementPolicy, ResourceScope, Service, ServiceRestart, ServiceStart, Shortcut,
    ShortcutLocation, SteamVrDriver,
};
use super::schema::{RawManifest, RawTarget};
use super::selector::selector;
use super::{Application, License, Manifest, Package, Update};
use crate::diagnostic::{self, Code, Diagnostic, Diagnostics, Location};
use crate::package::{Artifact, Os, PackageId, Version};

pub const SCHEMA: i64 = 1;

/// raw schema to checked manifest. every problem found becomes a
/// diagnostic; the manifest is only returned when none is an error.
pub(super) fn validate(
    raw: RawManifest,
    file: &str,
    source: &str,
) -> (Option<Manifest>, Diagnostics) {
    let mut v = Validator {
        file,
        source,
        diagnostics: Diagnostics::default(),
    };
    let manifest = v.manifest(raw);
    let manifest = (!v.diagnostics.has_errors()).then_some(manifest);
    (manifest, v.diagnostics)
}

struct Validator<'a> {
    file: &'a str,
    source: &'a str,
    diagnostics: Diagnostics,
}

impl Validator<'_> {
    fn at<T>(&self, spanned: &Spanned<T>) -> Location {
        Location::at(self.file, self.source, spanned.span().start)
    }

    fn error(&mut self, code: Code, at: Location, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(code, message).at(at));
    }

    /// parses a spanned string, reporting `field: error` at its span.
    fn value<T>(&mut self, field: &str, raw: &Spanned<String>) -> Option<T>
    where
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        match raw.get_ref().parse() {
            Ok(v) => Some(v),
            Err(e) => {
                let at = self.at(raw);
                self.error(diagnostic::MANIFEST_VALUE, at, format!("`{field}`: {e}"));
                None
            }
        }
    }

    fn keyword<T>(&mut self, field: &str, raw: Option<&Spanned<String>>, default: T) -> T
    where
        T: FromStr<Err = String>,
    {
        raw.and_then(|r| self.value(field, r)).unwrap_or(default)
    }

    fn manifest(&mut self, mut raw: RawManifest) -> Manifest {
        self.schema(raw.schema.as_ref());
        let package = self.package(&raw);
        let application = self.application(&raw);
        let install = self.install(&raw);
        let license = raw.license.as_ref().map(|l| License {
            file: l.file.get_ref().clone(),
            format: self.keyword("license.format", l.format.as_ref(), LicenseFormat::Text),
            require_acceptance: l.require_acceptance.unwrap_or(false),
        });
        let components = self.components(&raw);
        let files = self.files(&raw);
        let paths = self.paths(&raw);
        let steamvr_drivers = self.steamvr_drivers(&raw);
        let shortcuts = self.shortcuts(&raw);
        let services = self.services(&raw);
        let hooks = self.hooks(&raw);
        let requirements = self.requirements(&raw);
        let artifacts = self.targets(raw.targets.take());
        let update = raw.update.take().map(|u| Update {
            enabled: u.enabled.unwrap_or(false),
            channel: u.channel.unwrap_or_else(|| "stable".to_owned()),
            repository: u.repository.map(Spanned::into_inner),
        });
        let manifest = Manifest {
            package,
            application,
            install,
            license,
            components,
            files,
            paths,
            steamvr_drivers,
            shortcuts,
            services,
            hooks,
            requirements,
            update,
            artifacts,
            sources: Vec::new(),
            brand: super::Brand::default(),
            sign: None,
        };
        self.references(&manifest, &raw);
        manifest
    }

    fn schema(&mut self, schema: Option<&Spanned<i64>>) {
        match schema {
            None => self.diagnostics.push(
                Diagnostic::error(diagnostic::MANIFEST_SCHEMA, "manifest has no `schema`")
                    .at(Location::at(self.file, self.source, 0))
                    .help(format!("add `schema = {SCHEMA}` at the top")),
            ),
            Some(s) if *s.get_ref() == SCHEMA => {}
            Some(s) => {
                let message = format!(
                    "manifest schema {} is not supported, this kago reads schema {SCHEMA}",
                    s.get_ref()
                );
                let help = if *s.get_ref() > SCHEMA {
                    "update kago to read this manifest"
                } else {
                    "run `kago migrate` to upgrade the manifest"
                };
                let at = self.at(s);
                self.diagnostics.push(
                    Diagnostic::error(diagnostic::MANIFEST_SCHEMA, message)
                        .at(at)
                        .help(help),
                );
            }
        }
    }

    fn package(&mut self, raw: &RawManifest) -> Package {
        let p = &raw.package;
        let id = self.value::<PackageId>("package.id", &p.id);
        let version = self.value::<Version>("package.version", &p.version);
        Package {
            // placeholders only survive when there is already an error.
            id: id.unwrap_or_else(|| PackageId::new("invalid.id").expect("valid placeholder")),
            name: p.name.clone(),
            version: version.unwrap_or(Version::new(0, 0, 0)),
            publisher: p.publisher.clone(),
            description: p.description.clone(),
            homepage: p.homepage.clone(),
        }
    }

    fn application(&mut self, raw: &RawManifest) -> Application {
        let Some(a) = &raw.application else {
            return Application {
                kind: AppKind::Desktop,
                entrypoint: None,
                display_name: raw.package.name.clone(),
                description: raw.package.description.clone(),
                categories: Vec::new(),
                icons: Vec::new(),
            };
        };
        let icons = a
            .icons
            .iter()
            .map(|i| Icon {
                source: i.source.get_ref().clone(),
                size: i.size,
                when: self.when(i.when.as_ref()),
            })
            .collect();
        Application {
            kind: self.keyword("application.kind", a.kind.as_ref(), AppKind::Desktop),
            entrypoint: a.entrypoint.as_ref().map(|e| e.get_ref().clone()),
            display_name: a
                .display_name
                .clone()
                .unwrap_or_else(|| raw.package.name.clone()),
            description: a
                .description
                .clone()
                .or_else(|| raw.package.description.clone()),
            categories: a.categories.clone(),
            icons,
        }
    }

    fn install(&mut self, raw: &RawManifest) -> InstallPolicy {
        let d = InstallPolicy::default();
        let Some(i) = &raw.install else {
            return d;
        };
        InstallPolicy {
            scope: self.keyword("install.scope", i.scope.as_ref(), d.scope),
            scope_conflict: self.keyword(
                "install.scope_conflict",
                i.scope_conflict.as_ref(),
                d.scope_conflict,
            ),
            elevation: self.keyword("install.elevation", i.elevation.as_ref(), d.elevation),
            upgrade: self.keyword("install.upgrade", i.upgrade.as_ref(), d.upgrade),
            downgrade: self.keyword("install.downgrade", i.downgrade.as_ref(), d.downgrade),
            repair: i.repair.unwrap_or(d.repair),
            rollback: i.rollback.unwrap_or(d.rollback),
            look: self.keyword("install.look", i.look.as_ref(), d.look),
            folder: self.keyword("install.folder", i.folder.as_ref(), d.folder),
        }
    }

    fn when(&mut self, raw: Option<&super::schema::RawWhen>) -> super::Selector {
        selector(raw, self.file, self.source, &mut self.diagnostics)
    }

    fn components(&mut self, raw: &RawManifest) -> Vec<Component> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for c in &raw.components {
            if !seen.insert(c.id.get_ref().clone()) {
                let at = self.at(&c.id);
                self.error(
                    diagnostic::MANIFEST_DUPLICATE,
                    at,
                    format!("component `{}` is declared twice", c.id.get_ref()),
                );
            }
            let required = c.required.unwrap_or(false);
            out.push(Component {
                id: c.id.get_ref().clone(),
                name: c.name.clone(),
                description: c.description.clone(),
                required,
                default: required || c.default.unwrap_or(true),
                depends: c.depends.iter().map(|d| d.get_ref().clone()).collect(),
                detect: c
                    .detect
                    .as_ref()
                    .map(|d| self.keyword("component.detect", Some(d), Detect::SteamVr)),
            });
        }
        out
    }

    fn files(&mut self, raw: &RawManifest) -> Vec<File> {
        raw.files
            .iter()
            .filter_map(|f| {
                let at = self.at(&f.target);
                if f.source.get_ref().is_empty() {
                    let at = self.at(&f.source);
                    self.error(diagnostic::MANIFEST_PATH, at, "`files.source` is empty");
                }
                let target = self.path("files.target", &f.target);
                let ownership =
                    self.keyword("files.ownership", f.ownership.as_ref(), Ownership::Owned);
                let when = self.when(f.when.as_ref());
                Some(File {
                    logical_id: f.logical_id.as_ref().map(|l| l.get_ref().clone()),
                    component: f.component.as_ref().map(|c| c.get_ref().clone()),
                    source: f.source.get_ref().clone(),
                    target: target?,
                    ownership,
                    when,
                    at,
                })
            })
            .collect()
    }

    fn path(&mut self, field: &str, raw: &Spanned<String>) -> Option<SemanticPath> {
        match raw.get_ref().parse() {
            Ok(p) => Some(p),
            Err(e) => {
                let at = self.at(raw);
                self.error(diagnostic::MANIFEST_PATH, at, format!("`{field}`: {e}"));
                None
            }
        }
    }

    fn paths(&mut self, raw: &RawManifest) -> Vec<PathEntry> {
        raw.paths
            .iter()
            .filter_map(|spanned| {
                let p = spanned.get_ref();
                let at = self.at(spanned);
                let source = match (&p.resource, &p.directory) {
                    (Some(id), None) => PathSource::Resource {
                        id: id.get_ref().clone(),
                        kind: self.keyword(
                            "paths.kind",
                            p.kind.as_ref(),
                            PathKind::ExecutableParent,
                        ),
                    },
                    (None, Some(dir)) => PathSource::Directory(self.path("paths.directory", dir)?),
                    _ => {
                        self.error(
                            diagnostic::MANIFEST_VALUE,
                            at,
                            "`[[paths]]` needs exactly one of `resource` or `directory`",
                        );
                        return None;
                    }
                };
                Some(PathEntry {
                    source,
                    scope: self.keyword(
                        "paths.scope",
                        p.scope.as_ref(),
                        ResourceScope::ResolvedInstall,
                    ),
                    component: p.component.as_ref().map(|c| c.get_ref().clone()),
                    when: self.when(p.when.as_ref()),
                    at,
                })
            })
            .collect()
    }

    fn steamvr_drivers(&mut self, raw: &RawManifest) -> Vec<SteamVrDriver> {
        raw.steamvr_drivers
            .iter()
            .filter_map(|d| {
                Some(SteamVrDriver {
                    directory: self.path("steamvr", &d.directory)?,
                    component: d.component.as_ref().map(|c| c.get_ref().clone()),
                    when: self.when(d.when.as_ref()),
                    at: self.at(&d.directory),
                })
            })
            .collect()
    }

    fn shortcuts(&mut self, raw: &RawManifest) -> Vec<Shortcut> {
        raw.shortcuts
            .iter()
            .map(|s| Shortcut {
                name: s.name.clone(),
                target: s.target.get_ref().clone(),
                location: self.keyword(
                    "shortcuts.location",
                    s.location.as_ref(),
                    ShortcutLocation::Applications,
                ),
                arguments: s.arguments.clone(),
                component: s.component.as_ref().map(|c| c.get_ref().clone()),
                when: self.when(s.when.as_ref()),
                at: self.at(&s.target),
            })
            .collect()
    }

    fn services(&mut self, raw: &RawManifest) -> Vec<Service> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for s in &raw.services {
            if !seen.insert(s.id.get_ref().clone()) {
                let at = self.at(&s.id);
                self.error(
                    diagnostic::MANIFEST_DUPLICATE,
                    at,
                    format!("service `{}` is declared twice", s.id.get_ref()),
                );
            }
            out.push(Service {
                id: s.id.get_ref().clone(),
                name: s.name.clone(),
                command: s.command.get_ref().clone(),
                arguments: s.arguments.clone(),
                scope: self.keyword(
                    "services.scope",
                    s.scope.as_ref(),
                    ResourceScope::ResolvedInstall,
                ),
                start: self.keyword("services.start", s.start.as_ref(), ServiceStart::Automatic),
                restart: self.keyword(
                    "services.restart",
                    s.restart.as_ref(),
                    ServiceRestart::OnFailure,
                ),
                component: s.component.as_ref().map(|c| c.get_ref().clone()),
                when: self.when(s.when.as_ref()),
                at: self.at(&s.command),
            });
        }
        out
    }

    fn hooks(&mut self, raw: &RawManifest) -> Vec<Hook> {
        raw.hooks
            .iter()
            .filter_map(|h| {
                let phase = self.value("hooks.phase", &h.phase);
                let hook = Hook {
                    phase: phase?,
                    command: h.command.get_ref().clone(),
                    arguments: h.arguments.clone(),
                    privilege: self.keyword(
                        "hooks.privilege",
                        h.privilege.as_ref(),
                        HookPrivilege::User,
                    ),
                    when: self.when(h.when.as_ref()),
                    at: self.at(&h.command),
                };
                if hook.privilege == HookPrivilege::Elevated {
                    self.diagnostics.push(
                        Diagnostic::warning(
                            diagnostic::MANIFEST_VALUE,
                            "elevated hook runs arbitrary code as admin/root and weakens rollback",
                        )
                        .at(hook.at.clone())
                        .help("prefer a built-in operation if one fits"),
                    );
                }
                Some(hook)
            })
            .collect()
    }

    fn requirements(&mut self, raw: &RawManifest) -> Vec<Requirement> {
        raw.requirements
            .iter()
            .filter_map(|r| {
                let kind = self.value("requirements.kind", &r.kind);
                Some(Requirement {
                    id: r.id.get_ref().clone(),
                    kind: kind?,
                    policy: self.keyword(
                        "requirements.policy",
                        r.policy.as_ref(),
                        RequirementPolicy::Require,
                    ),
                    version: r.version.clone(),
                    minimum: r.minimum.clone(),
                    when: self.when(r.when.as_ref()),
                })
            })
            .collect()
    }

    fn targets(&mut self, raw: Option<super::schema::RawTargets>) -> BTreeMap<Os, Vec<Artifact>> {
        let mut artifacts = BTreeMap::new();
        let Some(raw) = raw else {
            return artifacts;
        };
        let all: [(Os, Option<RawTarget>); 3] = [
            (Os::Windows, raw.windows),
            (Os::Macos, raw.macos),
            (Os::Linux, raw.linux),
        ];
        for (os, target) in all {
            let Some(target) = target else { continue };
            let mut list = Vec::new();
            for a in &target.artifacts {
                let Some(artifact) = self.value::<Artifact>(&format!("targets.{os}.artifacts"), a)
                else {
                    continue;
                };
                if !artifact.supports(os) {
                    let at = self.at(a);
                    self.error(
                        diagnostic::TARGET_CAPABILITY,
                        at,
                        format!("`{artifact}` is not an artifact for {os}"),
                    );
                } else if !list.contains(&artifact) {
                    list.push(artifact);
                }
            }
            artifacts.insert(os, list);
        }
        artifacts
    }

    /// components, logical ids and their users all agree.
    fn references(&mut self, m: &Manifest, raw: &RawManifest) {
        let components: BTreeSet<&str> = m.components.iter().map(|c| c.id.as_str()).collect();
        for (c, rc) in m.components.iter().zip(&raw.components) {
            for (dep, rd) in c.depends.iter().zip(&rc.depends) {
                if dep == &c.id || !components.contains(dep.as_str()) {
                    let at = self.at(rd);
                    self.error(
                        diagnostic::MANIFEST_REFERENCE,
                        at,
                        format!("component `{}` depends on unknown component `{dep}`", c.id),
                    );
                }
            }
        }
        let component_uses = m
            .files
            .iter()
            .map(|f| (f.component.as_deref(), &f.at))
            .chain(m.paths.iter().map(|p| (p.component.as_deref(), &p.at)))
            .chain(m.shortcuts.iter().map(|s| (s.component.as_deref(), &s.at)))
            .chain(m.services.iter().map(|s| (s.component.as_deref(), &s.at)));
        let mut errors = Vec::new();
        for (component, at) in component_uses {
            if let Some(c) = component
                && !components.contains(c)
            {
                errors.push((at.clone(), format!("unknown component `{c}`")));
            }
        }

        let ids: BTreeSet<&str> = m
            .files
            .iter()
            .filter_map(|f| f.logical_id.as_deref())
            .collect();
        let mut resource = |what: &str, id: &str, at: &Location| {
            if !ids.contains(id) {
                errors.push((
                    at.clone(),
                    format!("{what} refers to `{id}`, which no `[[files]] logical_id` declares"),
                ));
            }
        };
        if let (Some(entry), Some(raw_entry)) = (
            &m.application.entrypoint,
            raw.application.as_ref().and_then(|a| a.entrypoint.as_ref()),
        ) {
            let at = Location::at(self.file, self.source, raw_entry.span().start);
            resource("`application.entrypoint`", entry, &at);
        }
        for s in &m.shortcuts {
            resource("shortcut", &s.target, &s.at);
        }
        for s in &m.services {
            resource("service", &s.command, &s.at);
        }
        for h in &m.hooks {
            resource("hook", &h.command, &h.at);
        }
        for p in &m.paths {
            if let PathSource::Resource { id, .. } = &p.source {
                resource("`[[paths]]`", id, &p.at);
            }
        }
        for (at, message) in errors {
            self.error(diagnostic::MANIFEST_REFERENCE, at, message);
        }
    }
}
