//! Kagofile blocks to the raw manifest the validator checks. every value
//! keeps the span of the word it came from, so later errors point into the
//! Kagofile.

use std::collections::BTreeMap;
use std::path::Path;

use serde_spanned::Spanned;

use super::tree::{Node, Word};
use super::version;
use crate::diagnostic::{self, Code, Diagnostic, Diagnostics, Location};
use crate::manifest::schema::{
    RawApplication, RawComponent, RawFile, RawHook, RawIcon, RawInstall, RawLicense, RawManifest,
    RawPackage, RawPath, RawRequirement, RawService, RawShortcut, RawSteamVrDriver, RawTarget,
    RawTargets, RawWhen,
};
use crate::manifest::source::{Source, SourceKind, Unpack};
use crate::manifest::{Brand, parse_color};
use crate::package::{Arch, Artifact, Libc, Os};

/// what one Kagofile lowers to.
pub struct Lowered {
    pub raw: RawManifest,
    pub sources: Vec<Source>,
    pub brand: Brand,
}

/// lowers `nodes`; `base` is the Kagofile directory, for `version from`.
pub fn lower(
    file: &str,
    source: &str,
    base: &Path,
    nodes: &[Node],
) -> (Option<Lowered>, Diagnostics) {
    let mut l = Lower {
        file,
        source,
        base,
        diagnostics: Diagnostics::default(),
        sources: Vec::new(),
        package: None,
        apps: Vec::new(),
        icons: Vec::new(),
        install: RawInstall {
            scope: None,
            scope_conflict: None,
            elevation: None,
            upgrade: None,
            downgrade: None,
            repair: None,
            rollback: None,
            look: None,
            folder: None,
        },
        license: None,
        components: Vec::new(),
        files: Vec::new(),
        paths: Vec::new(),
        steamvr_drivers: Vec::new(),
        requirements: Vec::new(),
        hooks: Vec::new(),
        services: Vec::new(),
        shortcuts: Vec::new(),
        targets: BTreeMap::new(),
        pending: Vec::new(),
        brand: Brand::default(),
    };
    // sources first, so references to them work in any order.
    for node in nodes.iter().filter(|n| n.keyword() == "source") {
        l.source_block(node);
    }
    for node in nodes {
        l.top(node);
    }
    l.finish()
}

/// a `path` or `shortcut` that needs the main app, resolved at the end.
enum Pending {
    Path {
        at: Word,
        on: bool,
        label: Option<String>,
        target: Option<Word>,
        scope: Scope,
    },
    Shortcut {
        at: Word,
        name: Option<String>,
        desktop: bool,
        target: Option<Word>,
        scope: Scope,
    },
}

/// component and `when` a line sits in.
#[derive(Clone, Default)]
struct Scope {
    component: Option<Spanned<String>>,
    when: Vec<Word>,
}

struct Lower<'a> {
    file: &'a str,
    source: &'a str,
    base: &'a Path,
    diagnostics: Diagnostics,
    sources: Vec<Source>,
    package: Option<RawPackage>,
    /// (id, span) of every app, the first is the entrypoint.
    apps: Vec<Word>,
    icons: Vec<RawIcon>,
    install: RawInstall,
    license: Option<RawLicense>,
    components: Vec<RawComponent>,
    files: Vec<RawFile>,
    paths: Vec<Spanned<RawPath>>,
    steamvr_drivers: Vec<RawSteamVrDriver>,
    requirements: Vec<RawRequirement>,
    hooks: Vec<RawHook>,
    services: Vec<RawService>,
    shortcuts: Vec<RawShortcut>,
    targets: BTreeMap<Os, Vec<Spanned<String>>>,
    pending: Vec<Pending>,
    brand: Brand,
}

fn spanned(word: &Word) -> Spanned<String> {
    Spanned::new(word.at..word.at + word.text.len().max(1), word.text.clone())
}

fn spanned_text(at: &Word, text: String) -> Spanned<String> {
    Spanned::new(at.at..at.at + 1, text)
}

impl Lower<'_> {
    fn error(&mut self, code: Code, at: usize, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(code, message).at(Location::at(self.file, self.source, at)));
    }

    fn syntax(&mut self, at: usize, message: impl Into<String>) {
        self.error(diagnostic::MANIFEST_SYNTAX, at, message);
    }

    /// exactly `n` arguments, or a syntax error naming the usage.
    fn args<'n>(&mut self, node: &'n Node, n: usize, usage: &str) -> Option<&'n [Word]> {
        if node.args().len() == n {
            Some(node.args())
        } else {
            self.syntax(node.at(), format!("expected `{usage}`"));
            None
        }
    }

    fn no_children(&mut self, node: &Node) {
        if let Some(child) = node.children.first() {
            self.syntax(
                child.at(),
                format!("`{}` takes no indented lines", node.keyword()),
            );
        }
    }

    fn top(&mut self, node: &Node) {
        match node.keyword() {
            "source" => {}
            "package" => self.package_block(node),
            "target" => self.target(node),
            "license" => self.license(node),
            "brand" => self.brand_block(node),
            _ => self.item(node, &Scope::default()),
        }
    }

    /// lines allowed at top level and inside `component` and `when`.
    fn item(&mut self, node: &Node, scope: &Scope) {
        match node.keyword() {
            "app" => self.app(node, scope),
            "copy" => self.copy(node, scope),
            "shortcut" => self.shortcut(node, scope),
            "path" => self.path(node, scope),
            "steamvr" => self.steamvr(node, scope),
            "require" => self.require(node, scope),
            "hook" => self.hook(node, scope),
            "service" => self.service(node, scope),
            "component" if scope.component.is_none() => self.component(node, scope),
            "when" => {
                let mut inner = scope.clone();
                inner.when.extend(node.args().iter().cloned());
                if node.args().is_empty() {
                    self.syntax(node.at(), "expected `when <os|arch|libc|format>...`");
                }
                for child in &node.children {
                    self.item(child, &inner);
                }
            }
            other => self.syntax(
                node.at(),
                format!(
                    "unknown statement `{other}`; expected package, source, app, copy, shortcut, path, steamvr, service, require, hook, component, when, target or license"
                ),
            ),
        }
    }

    fn package_block(&mut self, node: &Node) {
        if self.package.is_some() {
            self.error(
                diagnostic::MANIFEST_DUPLICATE,
                node.at(),
                "`package` is declared twice",
            );
            return;
        }
        let Some([id]) = self.args(node, 1, "package <id>") else {
            return;
        };
        let mut package = RawPackage {
            id: spanned(id),
            name: String::new(),
            version: spanned_text(id, String::new()),
            publisher: None,
            description: None,
            homepage: None,
        };
        let mut has_version = false;
        for child in &node.children {
            let text = rest(child.args());
            match child.keyword() {
                "name" => package.name = text,
                "publisher" => package.publisher = Some(text),
                "description" => package.description = Some(text),
                "homepage" => package.homepage = Some(text),
                "version" => {
                    has_version = true;
                    match version::read(child.args(), self.base) {
                        Ok(v) => package.version = spanned_text(&child.args()[0], v),
                        Err(e) => self.error(diagnostic::MANIFEST_VALUE, child.at(), e),
                    }
                }
                "scope" => self.install.scope = self.one(child),
                "conflict" => self.install.scope_conflict = self.one(child),
                "elevation" => self.install.elevation = self.one(child),
                "downgrade" => self.install.downgrade = self.one(child),
                "setup" => self.install.look = self.one(child),
                "folder" => self.install.folder = self.one(child),
                other => self.syntax(
                    child.at(),
                    format!(
                        "unknown package field `{other}`; expected name, version, publisher, description, homepage, scope, conflict, elevation, downgrade, setup or folder"
                    ),
                ),
            }
            if child.args().is_empty() {
                self.syntax(child.at(), format!("`{}` needs a value", child.keyword()));
            }
        }
        if package.name.is_empty() {
            // a later default: the last id segment.
            id.text
                .rsplit('.')
                .next()
                .unwrap_or(&id.text)
                .clone_into(&mut package.name);
        }
        if !has_version {
            self.error(
                diagnostic::MANIFEST_VALUE,
                node.at(),
                "`package` needs a `version` line, like `version 1.0.0` or `version from cargo`",
            );
        }
        self.package = Some(package);
    }

    fn one(&mut self, node: &Node) -> Option<Spanned<String>> {
        let [value] = self.args(node, 1, &format!("{} <value>", node.keyword()))? else {
            return None;
        };
        Some(spanned(value))
    }

    fn source_block(&mut self, node: &Node) {
        let Some([name]) = self.args(node, 1, "source <name>") else {
            return;
        };
        if name.text.len() < 2
            || !name
                .text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            self.error(
                diagnostic::MANIFEST_VALUE,
                name.at,
                "source names are two or more of a-z, 0-9, `-` and `_`",
            );
            return;
        }
        if self.sources.iter().any(|s| s.name == name.text) {
            self.error(
                diagnostic::MANIFEST_DUPLICATE,
                name.at,
                format!("source `{}` is declared twice", name.text),
            );
            return;
        }
        let mut kind = None;
        let mut sha256 = BTreeMap::new();
        let mut unpack = Unpack::Auto;
        for child in &node.children {
            match (child.keyword(), child.args()) {
                ("dir", [path]) => kind = Some(SourceKind::Dir(path.text.clone())),
                ("fetch", [url]) => kind = Some(SourceKind::Fetch(url.text.clone())),
                ("sha256", [hex]) => {
                    sha256.insert(None, hex.text.clone());
                }
                ("sha256", [target, hex]) => {
                    sha256.insert(Some(target.text.clone()), hex.text.clone());
                }
                ("unpack", [format]) => match format.text.parse() {
                    Ok(u) => unpack = u,
                    Err(e) => self.error(diagnostic::MANIFEST_VALUE, format.at, e),
                },
                _ => self.syntax(
                    child.at(),
                    "expected `dir <path>`, `fetch <url>`, `sha256 [target] <hex>` or `unpack auto|zip|tar.gz|none`",
                ),
            }
        }
        let Some(kind) = kind else {
            self.syntax(node.at(), "a source needs `dir <path>` or `fetch <url>`");
            return;
        };
        if matches!(kind, SourceKind::Fetch(_)) && sha256.is_empty() {
            // not an error yet: the first build downloads it once and names
            // the hash to pin, then refuses until it is pinned.
            self.diagnostics.push(
                Diagnostic::warning(
                    diagnostic::MANIFEST_VALUE,
                    format!(
                        "source `{}` has no `sha256`; builds will refuse it until it is pinned",
                        name.text
                    ),
                )
                .at(Location::at(self.file, self.source, node.at())),
            );
        }
        self.sources.push(Source {
            name: name.text.clone(),
            kind,
            sha256,
            unpack,
            at: Location::at(self.file, self.source, node.at()),
        });
    }

    /// `name:path` stays as written when `name` is a source; a plain path is
    /// relative to the Kagofile.
    fn reference(&mut self, word: &Word) -> Spanned<String> {
        if let Some((prefix, _)) = word.text.split_once(':')
            && prefix.len() > 1
            && !prefix.contains(['/', '\\'])
            && !self.sources.iter().any(|s| s.name == prefix)
        {
            self.error(
                diagnostic::MANIFEST_REFERENCE,
                word.at,
                format!("unknown source `{prefix}`"),
            );
        }
        spanned(word)
    }

    fn app(&mut self, node: &Node, scope: &Scope) {
        let Some([id]) = self.args(node, 1, "app <id>") else {
            return;
        };
        let mut to = "{app}".to_owned();
        let mut variants = 0;
        for child in &node.children {
            let args = child.args();
            match child.keyword() {
                "to" => match args {
                    [dir] => dir.text.trim_end_matches('/').clone_into(&mut to),
                    _ => self.syntax(child.at(), "expected `to <directory>`"),
                },
                "icon" => match args {
                    [os, path] => {
                        let when = self.when(scope, std::slice::from_ref(os));
                        let source = self.reference(path);
                        self.icons.push(RawIcon {
                            source,
                            size: None,
                            when,
                        });
                    }
                    _ => self.syntax(child.at(), "expected `icon <os> <file>`"),
                },
                _ => {
                    // `<os> [arch] [libc] <file> [-> <path>]`
                    let (args, dest) = match args {
                        [front @ .., arrow, dest] if arrow.text == "->" => (front, Some(dest)),
                        _ => (args, None),
                    };
                    let Some((path, selector)) = args.split_last() else {
                        self.syntax(
                            child.at(),
                            "expected `<os> [arch] <file>`, `icon <os> <file>` or `to <directory>`",
                        );
                        continue;
                    };
                    let selector: Vec<Word> = std::iter::once(child.words[0].clone())
                        .chain(selector.iter().cloned())
                        .collect();
                    variants += 1;
                    let when = self.when(scope, &selector);
                    let source = self.reference(path);
                    let target = match dest {
                        Some(dest) => spanned(dest),
                        None => Spanned::new(
                            path.at..path.at + path.text.len(),
                            format!("{to}/{}", basename(&path.text)),
                        ),
                    };
                    self.files.push(RawFile {
                        logical_id: Some(spanned(id)),
                        component: scope.component.clone(),
                        source,
                        target,
                        ownership: None,
                        when,
                    });
                }
            }
        }
        if variants == 0 {
            self.syntax(node.at(), "an app needs at least one `<os> <file>` line");
        }
        self.apps.push(id.clone());
    }

    fn copy(&mut self, node: &Node, scope: &Scope) {
        self.no_children(node);
        let args = node.args();
        let Some(from) = args.first() else {
            self.syntax(
                node.at(),
                "expected `copy <file|dir> [-> <path>] [as <id>] [keep|user-data]`",
            );
            return;
        };
        let mut target = None;
        let mut id = None;
        let mut ownership = None;
        let mut rest = args[1..].iter();
        while let Some(word) = rest.next() {
            match word.text.as_str() {
                "->" => target = rest.next(),
                "as" => id = rest.next(),
                "keep" => ownership = Some(spanned_text(word, "preserve-if-modified".to_owned())),
                "user-data" | "shared" | "external" | "generated" => {
                    ownership = Some(spanned(word));
                }
                _ => {
                    self.syntax(word.at, format!("unexpected `{}` in copy", word.text));
                    return;
                }
            }
        }
        let target = match target {
            Some(t) => spanned(t),
            None => Spanned::new(
                from.at..from.at + from.text.len(),
                format!("{{app}}/{}", basename(&from.text)),
            ),
        };
        let when = self.when(scope, &[]);
        let source = self.reference(from);
        self.files.push(RawFile {
            logical_id: id.map(spanned),
            component: scope.component.clone(),
            source,
            target,
            ownership,
            when,
        });
    }

    /// `service <app> [arguments...]`, with `name "..."` and
    /// `scope user|machine` lines under it.
    fn service(&mut self, node: &Node, scope: &Scope) {
        let [app, arguments @ ..] = node.args() else {
            self.syntax(node.at(), "expected `service <app> [arguments...]`");
            return;
        };
        let mut name = None;
        let mut service_scope = None;
        for child in &node.children {
            match child.keyword() {
                "name" => name = Some(rest(child.args())),
                "scope" => service_scope = self.one(child),
                other => self.syntax(
                    child.at(),
                    format!("unknown service field `{other}`; expected name or scope"),
                ),
            }
        }
        let when = self.when(scope, &[]);
        self.services.push(RawService {
            id: spanned(app),
            name,
            command: spanned(app),
            arguments: arguments.iter().map(|a| a.text.clone()).collect(),
            scope: service_scope,
            start: None,
            restart: None,
            component: scope.component.clone(),
            when,
        });
    }

    /// `hook <phase> <app> [arguments...]`: runs one of the package's own
    /// programs at an install phase.
    fn hook(&mut self, node: &Node, scope: &Scope) {
        self.no_children(node);
        let [phase, app, arguments @ ..] = node.args() else {
            self.syntax(
                node.at(),
                "expected `hook post-install|pre-uninstall <app> [arguments...]`",
            );
            return;
        };
        let when = self.when(scope, &[]);
        self.hooks.push(RawHook {
            phase: spanned(phase),
            command: spanned(app),
            arguments: arguments.iter().map(|a| a.text.clone()).collect(),
            privilege: None,
            when,
        });
    }

    /// `require <os> <minimum>` or `require <software>`, with
    /// `install-if-missing` to ask for more than a check.
    fn require(&mut self, node: &Node, scope: &Scope) {
        self.no_children(node);
        let usage = "expected `require windows 10.0.19041` or `require steamvr`";
        let mut words: Vec<&Word> = node.args().iter().collect();
        let policy = match words.last() {
            Some(w) if w.text == "install-if-missing" && !w.quoted => words.pop().map(spanned),
            _ => None,
        };
        let Some((what, rest)) = words.split_first() else {
            self.syntax(node.at(), usage);
            return;
        };
        let is_os = what.text.parse::<Os>().is_ok();
        let minimum = match (is_os, rest) {
            (true, [minimum]) => Some(minimum.text.clone()),
            (false, []) => None,
            _ => {
                self.syntax(node.at(), usage);
                return;
            }
        };
        let os_word = (*what).clone();
        let when = if is_os {
            self.when(scope, std::slice::from_ref(&os_word))
        } else {
            self.when(scope, &[])
        };
        let kind = if is_os { "os" } else { "runtime" };
        self.requirements.push(RawRequirement {
            id: spanned(what),
            kind: spanned_text(what, kind.to_owned()),
            policy,
            version: None,
            minimum,
            when,
        });
    }

    /// `steamvr <dir> [-> <path>]`: copies a SteamVR driver folder and
    /// registers it with SteamVR, by default as `{app}/steamvr/<dir name>`.
    fn steamvr(&mut self, node: &Node, scope: &Scope) {
        self.no_children(node);
        let usage = "expected `steamvr <driver dir> [-> <path>]`";
        let args = node.args();
        let (from, target) = match args {
            [from] => (from, None),
            [from, arrow, to] if arrow.text == "->" && !arrow.quoted => (from, Some(to)),
            _ => {
                self.syntax(node.at(), usage);
                return;
            }
        };
        let target = match target {
            Some(t) => spanned(t),
            None => Spanned::new(
                from.at..from.at + from.text.len(),
                format!("{{app}}/steamvr/{}", basename(&from.text)),
            ),
        };
        let source = self.reference(from);
        let when = self.when(scope, &[]);
        self.files.push(RawFile {
            logical_id: None,
            component: scope.component.clone(),
            source,
            target: target.clone(),
            ownership: None,
            when,
        });
        let when = self.when(scope, &[]);
        self.steamvr_drivers.push(RawSteamVrDriver {
            directory: target,
            component: scope.component.clone(),
            when,
        });
    }

    fn shortcut(&mut self, node: &Node, scope: &Scope) {
        self.no_children(node);
        let mut name = None;
        let mut desktop = false;
        let mut target = None;
        let mut args = node.args().iter();
        while let Some(word) = args.next() {
            match word.text.as_str() {
                "desktop" if !word.quoted => desktop = true,
                "->" if !word.quoted => target = args.next().cloned(),
                _ if name.is_none() => name = Some(word.text.clone()),
                _ => self.syntax(
                    word.at,
                    "expected `shortcut [\"Name\"] [desktop] [-> <app>]`",
                ),
            }
        }
        self.pending.push(Pending::Shortcut {
            at: node.words[0].clone(),
            name,
            desktop,
            target,
            scope: scope.clone(),
        });
    }

    fn path(&mut self, node: &Node, scope: &Scope) {
        self.no_children(node);
        let mut on = true;
        let mut label = None;
        let mut target = None;
        let mut args = node.args().iter();
        while let Some(word) = args.next() {
            match word.text.as_str() {
                "on" if !word.quoted => on = true,
                "optional" if !word.quoted => on = false,
                "->" if !word.quoted => target = args.next().cloned(),
                _ if label.is_none() => label = Some(word.text.clone()),
                _ => self.syntax(
                    word.at,
                    "expected `path [on|optional] [\"label\"] [-> <app>]`",
                ),
            }
        }
        self.pending.push(Pending::Path {
            at: node.words[0].clone(),
            on,
            label,
            target,
            scope: scope.clone(),
        });
    }

    fn component(&mut self, node: &Node, scope: &Scope) {
        let args = node.args();
        let Some(id) = args.first() else {
            self.syntax(
                node.at(),
                "expected `component <id> [\"Name\"] [on|off|required]`",
            );
            return;
        };
        let mut name = id.text.clone();
        let (mut default, mut required) = (true, false);
        let mut depends = Vec::new();
        let mut words = args[1..].iter();
        while let Some(word) = words.next() {
            match word.text.as_str() {
                "on" if !word.quoted => default = true,
                "off" if !word.quoted => default = false,
                "required" if !word.quoted => required = true,
                "needs" if !word.quoted => match words.next() {
                    Some(other) => depends.push(spanned(other)),
                    None => self.syntax(word.at, "expected `needs <component>`"),
                },
                _ => name.clone_from(&word.text),
            }
        }
        let inner = Scope {
            component: Some(spanned(id)),
            when: scope.when.clone(),
        };
        let mut description = None;
        let mut detect = None;
        for child in &node.children {
            match child.keyword() {
                "about" if child.args().is_empty() => {
                    self.syntax(child.at(), "expected `about <text>`");
                }
                "about" => description = Some(rest(child.args())),
                "detect" => {
                    self.no_children(child);
                    detect = self.one(child);
                }
                _ => self.item(child, &inner),
            }
        }
        self.components.push(RawComponent {
            id: spanned(id),
            name,
            description,
            required: Some(required),
            default: Some(default),
            depends,
            detect,
        });
    }

    fn target(&mut self, node: &Node) {
        self.no_children(node);
        let Some((os, artifacts)) = node.args().split_first() else {
            self.syntax(node.at(), "expected `target <os> <artifact>...`");
            return;
        };
        let os = match os.text.parse::<Os>() {
            Ok(os) => os,
            Err(e) => return self.error(diagnostic::MANIFEST_VALUE, os.at, e.to_string()),
        };
        if artifacts.is_empty() {
            self.syntax(node.at(), "expected `target <os> <artifact>...`");
        }
        self.targets
            .entry(os)
            .or_default()
            .extend(artifacts.iter().map(spanned));
    }

    /// `brand` with `text`, `accent`, `logo`, `header`, `side`, `welcome`,
    /// `finish` and `link` lines. colors go without `#`, which starts a
    /// comment: `accent 6667ab`.
    fn brand_block(&mut self, node: &Node) {
        if !node.args().is_empty() {
            self.syntax(
                node.at(),
                "`brand` takes its settings on the lines under it",
            );
        }
        for child in &node.children {
            self.no_children(child);
            let args = child.args();
            if args.is_empty() {
                self.syntax(child.at(), format!("`{}` needs a value", child.keyword()));
                continue;
            }
            let image = |l: &mut Self| {
                if let [file] = args {
                    Some(l.reference(file).into_inner())
                } else {
                    l.syntax(
                        child.at(),
                        format!("`{}` takes one PNG file", child.keyword()),
                    );
                    None
                }
            };
            match child.keyword() {
                "text" => self.brand.text = Some(rest(args)),
                "welcome" => self.brand.welcome = Some(rest(args)),
                "finish" => self.brand.finish = Some(rest(args)),
                "accent" => match parse_color(&args[0].text) {
                    Ok(rgb) => self.brand.accent = Some(rgb),
                    Err(e) => self.error(diagnostic::MANIFEST_VALUE, args[0].at, e),
                },
                "logo" => self.brand.logo = image(self),
                "header" => self.brand.header = image(self),
                "side" => self.brand.side = image(self),
                "link" => match args {
                    [label, url] => {
                        self.brand.link = Some((label.text.clone(), url.text.clone()));
                    }
                    _ => self.syntax(child.at(), "expected `link \"Label\" <url>`"),
                },
                other => self.syntax(
                    child.at(),
                    format!(
                        "unknown brand field `{other}`; expected text, accent, logo, header, side, welcome, finish or link"
                    ),
                ),
            }
        }
    }

    fn license(&mut self, node: &Node) {
        self.no_children(node);
        match node.args() {
            [file] => {
                self.license = Some(RawLicense {
                    file: spanned(file),
                    format: None,
                    require_acceptance: Some(false),
                });
            }
            [file, accept] if accept.text == "accept" => {
                self.license = Some(RawLicense {
                    file: spanned(file),
                    format: None,
                    require_acceptance: Some(true),
                });
            }
            _ => self.syntax(node.at(), "expected `license <file> [accept]`"),
        }
    }

    /// selector words to the raw `when`, each word sorted by what it names.
    fn when(&mut self, scope: &Scope, extra: &[Word]) -> Option<RawWhen> {
        let words: Vec<&Word> = scope.when.iter().chain(extra).collect();
        if words.is_empty() {
            return None;
        }
        let mut keys: BTreeMap<&str, (usize, Vec<String>)> = BTreeMap::new();
        for w in words {
            let key = if w.text.parse::<Os>().is_ok() {
                "os"
            } else if w.text.parse::<Arch>().is_ok() {
                "arch"
            } else if w.text.parse::<Libc>().is_ok() {
                "libc"
            } else if w.text.parse::<Artifact>().is_ok() {
                "format"
            } else {
                self.error(
                    diagnostic::MANIFEST_SELECTOR,
                    w.at,
                    format!("`{}` is not an os, arch, libc or format", w.text),
                );
                continue;
            };
            let entry = keys.entry(key).or_insert((w.at, Vec::new()));
            entry.1.push(w.text.clone());
        }
        let mut take = |key: &str| {
            keys.remove(key)
                .map(|(at, values)| Spanned::new(at..at + 1, values))
        };
        Some(RawWhen {
            os: take("os"),
            arch: take("arch"),
            libc: take("libc"),
            format: take("format"),
            profile: None,
            channel: None,
        })
    }

    /// shortcuts and PATH entries point at the main app unless told
    /// otherwise; the main app is only known once every line was read.
    fn resolve_pending(&mut self) {
        let main = self.apps.first().cloned();
        let package_name = self
            .package
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_default();
        for pending in std::mem::take(&mut self.pending) {
            match pending {
                Pending::Shortcut {
                    at,
                    name,
                    desktop,
                    target,
                    scope,
                } => {
                    let Some(app) = target.or_else(|| main.clone()) else {
                        self.syntax(at.at, "`shortcut` needs an `app` to point at");
                        continue;
                    };
                    let when = self.when(&scope, &[]);
                    self.shortcuts.push(RawShortcut {
                        name: name.unwrap_or_else(|| package_name.clone()),
                        target: spanned(&app),
                        location: desktop.then(|| spanned_text(&at, "desktop".to_owned())),
                        arguments: Vec::new(),
                        component: scope.component,
                        when,
                    });
                }
                Pending::Path {
                    at,
                    on,
                    label,
                    target,
                    scope,
                } => {
                    let Some(app) = target.or_else(|| main.clone()) else {
                        self.syntax(at.at, "`path` needs an `app` to point at");
                        continue;
                    };
                    let mut component = scope.component.clone();
                    if !on && component.is_none() {
                        let id = format!("path-{}", app.text);
                        self.components.push(RawComponent {
                            id: spanned_text(&at, id.clone()),
                            name: label.unwrap_or_else(|| format!("Add {} to PATH", app.text)),
                            description: None,
                            required: Some(false),
                            default: Some(false),
                            depends: Vec::new(),
                            detect: None,
                        });
                        component = Some(spanned_text(&at, id));
                    }
                    let when = self.when(&scope, &[]);
                    self.paths.push(Spanned::new(
                        at.at..at.at + 4,
                        RawPath {
                            resource: Some(spanned(&app)),
                            directory: None,
                            kind: None,
                            scope: None,
                            component,
                            when,
                        },
                    ));
                }
            }
        }
    }

    fn finish(mut self) -> (Option<Lowered>, Diagnostics) {
        self.resolve_pending();
        let Some(package) = self.package.take() else {
            self.error(
                diagnostic::MANIFEST_VALUE,
                0,
                "a Kagofile needs a `package <id>` block with at least a `version`",
            );
            return (None, self.diagnostics);
        };
        if self.targets.is_empty() {
            // a Windows setup unless the Kagofile says otherwise.
            self.targets
                .insert(Os::Windows, vec![Spanned::new(0..1, "setup".to_owned())]);
        }
        let target =
            |list: Option<Vec<Spanned<String>>>| list.map(|artifacts| RawTarget { artifacts });
        let raw = RawManifest {
            schema: Some(Spanned::new(0..1, crate::manifest::SCHEMA)),
            package,
            application: Some(RawApplication {
                kind: None,
                entrypoint: self.apps.first().map(spanned),
                display_name: None,
                description: None,
                categories: Vec::new(),
                icons: self.icons,
            }),
            install: Some(self.install),
            license: self.license,
            components: self.components,
            files: self.files,
            paths: self.paths,
            steamvr_drivers: self.steamvr_drivers,
            shortcuts: self.shortcuts,
            services: self.services,
            hooks: self.hooks,
            requirements: self.requirements,
            update: None,
            targets: Some(RawTargets {
                windows: target(self.targets.remove(&Os::Windows)),
                macos: target(self.targets.remove(&Os::Macos)),
                linux: target(self.targets.remove(&Os::Linux)),
            }),
        };
        let lowered = Lowered {
            raw,
            sources: self.sources,
            brand: self.brand,
        };
        (
            (!self.diagnostics.has_errors()).then_some(lowered),
            self.diagnostics,
        )
    }
}

/// words joined back with single spaces, for names and descriptions.
fn rest(words: &[Word]) -> String {
    words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// last path segment of a reference, after any `source:`.
fn basename(reference: &str) -> &str {
    let path = reference.split_once(':').map_or(reference, |(_, p)| p);
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
}
