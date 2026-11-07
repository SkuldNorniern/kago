//! Kagofile blocks to the raw manifest. every value keeps the span of its
//! word, so later errors point into the Kagofile.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_spanned::Spanned;

use super::action::{self, Action};
use super::tree::{Node, Word};
use super::version;
use crate::diagnostic::{self, Code, Diagnostic, Diagnostics, Location};
use crate::manifest::schema::{
    RawApplication, RawComponent, RawFile, RawIcon, RawInstall, RawLicense, RawManifest,
    RawPackage, RawPath, RawRequirement, RawService, RawShortcut, RawTarget, RawTargets, RawUpdate,
    RawWhen,
};
use crate::manifest::source::{Source, SourceKind, Unpack};
use crate::manifest::{
    Brand, BuildStep, Change, Check, Command, Detect, Driver, Edit, Expect, Field, FieldKind,
    FirewallRule, InputPage, Place, Prerequisite, Run, RunAt, Setting, Sign, SignKey, TIMESTAMP,
    Value, parse_color,
};
use crate::package::{Arch, Artifact, Libc, Os};

/// the statements kago knows; anything else is an action.
const STATEMENTS: &[&str] = &[
    "package",
    "source",
    "app",
    "copy",
    "shortcut",
    "path",
    "service",
    "require",
    "run",
    "component",
    "when",
    "target",
    "license",
    "brand",
    "sign",
    "prerequisite",
    "update",
    "firewall",
    "driver",
    "json",
    "registry",
    "env",
    "check",
    "action",
    "use",
    "expect",
    "build",
    "page",
    "purge",
];

/// kago's own checks and actions, written in Kagofile syntax.
const LIBRARY: &str = include_str!("library.kago");

/// what one Kagofile lowers to.
pub struct Lowered {
    pub raw: RawManifest,
    pub sources: Vec<Source>,
    pub brand: Brand,
    pub sign: Option<Sign>,
    pub prerequisites: Vec<Prerequisite>,
    pub firewall: Vec<FirewallRule>,
    pub drivers: Vec<Driver>,
    pub changes: Vec<Change>,
    pub expects: Vec<Expect>,
    pub runs: Vec<Run>,
    pub build: Vec<BuildStep>,
    pub inputs: Vec<InputPage>,
    pub purge: Vec<String>,
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
        requirements: Vec::new(),
        runs: Vec::new(),
        build: Vec::new(),
        inputs: Vec::new(),
        purge: Vec::new(),
        services: Vec::new(),
        shortcuts: Vec::new(),
        targets: BTreeMap::new(),
        pending: Vec::new(),
        brand: Brand::default(),
        sign: None,
        prerequisites: Vec::new(),
        update: None,
        firewall: Vec::new(),
        drivers: Vec::new(),
        changes: Vec::new(),
        checks: BTreeMap::new(),
        library_checks: BTreeSet::new(),
        actions: BTreeMap::new(),
        library_actions: BTreeSet::new(),
        depth: 0,
        expects: Vec::new(),
    };
    // sources and checks first, so references to them work in any
    // order. the library's checks come before the Kagofile's own, which
    // may replace them.
    for node in nodes.iter().filter(|n| n.keyword() == "source") {
        l.source_block(node);
    }
    let library = super::tree::parse("kago library", LIBRARY).expect("the library parses");
    let library: Vec<Node> = library.iter().map(|n| action::moved(n, 0)).collect();
    l.definitions(&library);
    l.library_checks = l.checks.keys().cloned().collect();
    l.library_actions = l.actions.keys().cloned().collect();
    for node in nodes.iter().filter(|n| n.keyword() == "use") {
        l.use_file(node);
    }
    l.definitions(nodes);
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
    requirements: Vec<RawRequirement>,
    runs: Vec<Run>,
    build: Vec<BuildStep>,
    inputs: Vec<InputPage>,
    purge: Vec<String>,
    services: Vec<RawService>,
    shortcuts: Vec<RawShortcut>,
    targets: BTreeMap<Os, Vec<Spanned<String>>>,
    pending: Vec<Pending>,
    brand: Brand,
    sign: Option<Sign>,
    prerequisites: Vec<Prerequisite>,
    update: Option<RawUpdate>,
    firewall: Vec<FirewallRule>,
    drivers: Vec<Driver>,
    changes: Vec<Change>,
    /// `check` definitions by id.
    checks: BTreeMap<String, Detect>,
    /// ids of the checks that came from the library, which a Kagofile may
    /// define again.
    library_checks: BTreeSet<String>,
    actions: BTreeMap<String, Action>,
    /// names of the actions from the library, which a Kagofile may define
    /// again.
    library_actions: BTreeSet<String>,
    /// how many action calls deep the current line is.
    depth: usize,
    expects: Vec<Expect>,
}

/// `<text>` or `env <VARIABLE>`.
fn setting(words: &[&str]) -> Option<Setting> {
    match words {
        ["env", name] => Some(Setting::Env((*name).to_owned())),
        [text] if *text != "env" => Some(Setting::Text((*text).to_owned())),
        _ => None,
    }
}

fn sign_usage(field: &str) -> &'static str {
    match field {
        "certificate" => "expected `certificate [machine] <thumbprint>` or `certificate env <VAR>`",
        "pfx" => {
            "expected `pfx <file.pfx> [env <VAR>]`: a password comes from the environment, never the Kagofile"
        }
        "azure" => "expected `azure <metadata.json> dlib <path to Azure.CodeSigning.Dlib.dll>`",
        "timestamp" => "expected `timestamp <url>`",
        _ => "expected `programs on` or `programs off`",
    }
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
            "source" | "check" | "action" | "use" => {}
            "package" => self.package_block(node),
            "target" => self.target(node),
            "license" => self.license(node),
            "brand" => self.brand_block(node),
            "sign" => self.sign_block(node),
            "prerequisite" => self.prerequisite(node),
            "update" => self.update(node),
            "page" => self.page(node),
            "purge" => self.purge_line(node),
            "build" => self.build_block(node),
            name if self.actions.contains_key(name) => self.call(node, &Scope::default(), true),
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
            "expect" => self.expect(node, scope),
            "require" => self.require(node, scope),
            "run" => self.run(node, scope),
            "hook" => self.syntax(
                node.at(),
                "`hook` is now `run <program> [arguments]`, with `at uninstall` under it to run at uninstall",
            ),
            "service" => self.service(node, scope),
            "firewall" => self.firewall_rule(node, scope),
            "driver" => self.driver(node, scope),
            "json" | "registry" | "env" => self.change(node, scope),
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
            name if self.actions.contains_key(name) => self.call(node, scope, false),
            other => self.syntax(
                node.at(),
                format!(
                    "unknown statement `{other}`; expected {}, or an action like {}",
                    STATEMENTS.join(", "),
                    self.actions.keys().cloned().collect::<Vec<_>>().join(", ")
                ),
            ),
        }
    }

    /// the `check` and `action` definitions among `nodes`.
    fn definitions(&mut self, nodes: &[Node]) {
        for node in nodes {
            match node.keyword() {
                "check" => self.check_block(node),
                "action" => self.action(node),
                _ => {}
            }
        }
    }

    fn action(&mut self, node: &Node) {
        let (name, defined) = match Action::define(node) {
            Ok(a) => a,
            Err((at, e)) => return self.syntax(at, e),
        };
        let at = node.args().first().map_or(node.at(), |w| w.at);
        if STATEMENTS.contains(&name.as_str()) {
            return self.error(
                diagnostic::MANIFEST_VALUE,
                at,
                format!("`{name}` is a statement already; name the action something else"),
            );
        }
        if self.actions.contains_key(&name) && !self.library_actions.remove(&name) {
            return self.error(
                diagnostic::MANIFEST_DUPLICATE,
                at,
                format!("action `{name}` is declared twice"),
            );
        }
        self.actions.insert(name, defined);
    }

    /// `use <file>`: the checks and actions in another Kagofile-syntax
    /// file, relative to this one.
    fn use_file(&mut self, node: &Node) {
        self.no_children(node);
        let Some([path]) = self.args(node, 1, "use <file>") else {
            return;
        };
        let file = self.base.join(&path.text);
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) => {
                return self.error(
                    diagnostic::MANIFEST_REFERENCE,
                    path.at,
                    format!("cannot read {}: {e}", file.display()),
                );
            }
        };
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let nodes = match super::tree::parse(&file.display().to_string(), text) {
            Ok(nodes) => nodes,
            Err(d) => return self.diagnostics.push(d),
        };
        let mut used = Vec::new();
        for node in &nodes {
            match node.keyword() {
                "check" | "action" => used.push(action::moved(node, path.at)),
                other => self.syntax(
                    path.at,
                    format!(
                        "{} has `{other}`; a used file holds only `check` and `action`",
                        file.display()
                    ),
                ),
            }
        }
        self.definitions(&used);
    }

    /// an action's lines in place of the call, as if written there.
    fn call(&mut self, node: &Node, scope: &Scope, top: bool) {
        let name = node.keyword();
        let defined = &self.actions[name];
        if let Some(child) = node.children.first() {
            return self.syntax(
                child.at(),
                format!("`{}` takes no indented lines", defined.usage(name)),
            );
        }
        if self.depth >= 16 {
            return self.syntax(
                node.at(),
                format!("`{name}` calls actions 16 deep; does it call itself?"),
            );
        }
        let lines = match defined.call(node.args(), node.at()) {
            Ok(lines) => lines,
            Err((at, e)) => {
                let usage = defined.usage(name);
                return self.syntax(at, format!("{e}; expected `{usage}`"));
            }
        };
        self.depth += 1;
        for line in &lines {
            if top {
                self.top(line);
            } else {
                self.item(line, scope);
            }
        }
        self.depth -= 1;
    }

    /// `page <id> "Title"` with `about <text>` and field lines:
    /// `text|number|choice|check <id> "Label"` and their options.
    fn page(&mut self, node: &Node) {
        let (id, title) = match node.args() {
            [id, title] => (id, title.text.clone()),
            [id] => (id, id.text.clone()),
            _ => return self.syntax(node.at(), "expected `page <id> [\"Title\"]`"),
        };
        let mut page = InputPage {
            id: id.text.clone(),
            title,
            about: None,
            fields: Vec::new(),
            skip: None,
        };
        for child in &node.children {
            self.no_children(child);
            match child.keyword() {
                "about" => page.about = Some(rest(child.args())),
                "skip" => page.skip = self.detect(child.args(), child.at()),
                _ => {
                    if let Some(field) = self.field(child) {
                        let taken = self
                            .inputs
                            .iter()
                            .chain(std::iter::once(&page))
                            .flat_map(|p| &p.fields)
                            .any(|f| f.id == field.id);
                        if taken {
                            self.error(
                                diagnostic::MANIFEST_DUPLICATE,
                                child.at(),
                                format!("field `{}` is declared twice", field.id),
                            );
                        } else {
                            page.fields.push(field);
                        }
                    }
                }
            }
        }
        if page.fields.is_empty() {
            return self.syntax(
                node.at(),
                "a page needs at least one field, like `text host \"Address\"`",
            );
        }
        self.inputs.push(page);
    }

    /// one field line: kind, id, label, then options.
    fn field(&mut self, node: &Node) -> Option<Field> {
        let usage =
            "expected `text|number|choice|check <id> \"Label\" [options]`, or `about <text>`";
        let [id, rest_words @ ..] = node.args() else {
            self.syntax(node.at(), usage);
            return None;
        };
        let valid = !id.text.is_empty()
            && id
                .text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid {
            self.error(
                diagnostic::MANIFEST_VALUE,
                id.at,
                format!("`{}` is not a field id; use a-z, 0-9, `-` and `_`", id.text),
            );
            return None;
        }
        let (label, options) = match rest_words {
            [label, rest @ ..] if label.quoted => (label.text.clone(), rest),
            rest => (id.text.clone(), rest),
        };
        let mut kind = match node.keyword() {
            "text" => FieldKind::Text { secret: false },
            "number" => FieldKind::Number {
                min: None,
                max: None,
            },
            "choice" => FieldKind::Choice {
                options: Vec::new(),
            },
            "check" => FieldKind::Check,
            _ => {
                self.syntax(node.at(), usage);
                return None;
            }
        };
        let mut default = String::new();
        let mut required = false;
        for word in options {
            let text = word.text.as_str();
            let number = |v: &str| v.parse::<i64>().ok();
            match (&mut kind, text.split_once('=')) {
                (_, Some(("default", v))) => v.clone_into(&mut default),
                (FieldKind::Number { min, .. }, Some(("min", v))) if number(v).is_some() => {
                    *min = number(v);
                }
                (FieldKind::Number { max, .. }, Some(("max", v))) if number(v).is_some() => {
                    *max = number(v);
                }
                (FieldKind::Text { secret }, None) if text == "secret" => *secret = true,
                (FieldKind::Check, None) if text == "on" || text == "off" => {
                    default = (text == "on").to_string();
                }
                (FieldKind::Choice { options }, None) if !word.quoted || !text.is_empty() => {
                    if text == "required" {
                        required = true;
                    } else {
                        options.push(text.to_owned());
                    }
                }
                (_, None) if text == "required" => required = true,
                _ => {
                    self.syntax(
                        word.at,
                        format!("`{text}` is not an option of `{}`", node.keyword()),
                    );
                    return None;
                }
            }
        }
        match &kind {
            FieldKind::Choice { options } if options.is_empty() => {
                self.syntax(
                    node.at(),
                    "a choice needs its options, like `choice mode \"Mode\" local cloud`",
                );
                return None;
            }
            FieldKind::Choice { options } if default.is_empty() => default.clone_from(&options[0]),
            FieldKind::Check if default.is_empty() => "false".clone_into(&mut default),
            _ => {}
        }
        let field = Field {
            id: id.text.clone(),
            label,
            kind,
            default,
            required: false,
        };
        if let Err(problem) = field.check(&field.default) {
            self.error(
                diagnostic::MANIFEST_VALUE,
                node.at(),
                format!("default `{}` does not fit: {problem}", field.default),
            );
            return None;
        }
        Some(Field { required, ..field })
    }

    /// `build <program> [arguments]` or `build script <file> [arguments]`,
    /// or `build` with such lines under it, which `when` blocks may hold.
    fn build_block(&mut self, node: &Node) {
        if node.args().is_empty() && node.children.is_empty() {
            return self.syntax(
                node.at(),
                "expected `build <program> [arguments]`, `build script <file>`, or lines under `build`",
            );
        }
        if !node.args().is_empty() {
            if let Some(child) = node.children.first() {
                return self.syntax(
                    child.at(),
                    "put every step under `build`, or one on its line",
                );
            }
            self.build_step(node.args(), node.at(), &[]);
            return;
        }
        self.build_lines(&node.children, &[]);
    }

    fn build_lines(&mut self, lines: &[Node], when: &[Word]) {
        for line in lines {
            if line.keyword() == "when" {
                if line.args().is_empty() {
                    self.syntax(line.at(), "expected `when <os|arch|libc|format>...`");
                }
                let inner: Vec<Word> = when.iter().chain(line.args()).cloned().collect();
                self.build_lines(&line.children, &inner);
            } else {
                self.no_children(line);
                self.build_step(&line.words, line.at(), when);
            }
        }
    }

    /// one step from its words: `script <file> [arguments]` or
    /// `<program> [arguments]`.
    fn build_step(&mut self, words: &[Word], at: usize, when: &[Word]) {
        let (script, words) = match words {
            [first, rest @ ..] if first.text == "script" && !first.quoted => (true, rest),
            _ => (false, words),
        };
        let Some(command) = invocation(words) else {
            return self.syntax(at, "expected `script <file> [arguments]`");
        };
        if script && !self.base.join(&command.program).is_file() {
            self.error(
                diagnostic::MANIFEST_REFERENCE,
                at,
                format!(
                    "build script {} does not exist",
                    self.base.join(&command.program).display()
                ),
            );
            return;
        }
        let when = self.when(&Scope::default(), when);
        let when = crate::manifest::selector::selector(
            when.as_ref(),
            self.file,
            self.source,
            &mut self.diagnostics,
        );
        self.build.push(BuildStep {
            command,
            script,
            when,
        });
    }

    /// `purge <folder>`: a folder of the app's own data, removed when the
    /// person asks uninstall to remove their data too.
    fn purge_line(&mut self, node: &Node) {
        self.no_children(node);
        let Some([folder]) = self.args(node, 1, "purge <folder>") else {
            return;
        };
        if let Err(e) = location(&folder.text) {
            return self.error(diagnostic::MANIFEST_VALUE, folder.at, e);
        }
        // never a whole known folder, like `%APPDATA%` itself.
        // what follows `{root}` or `%VARIABLE%`.
        let after = match folder.text.strip_prefix('%') {
            Some(rest) => rest.split_once('%').map(|(_, after)| after),
            None => folder.text.split_once('}').map(|(_, after)| after),
        };
        // a real folder name, never `.`, which is the known folder itself.
        let below = after.is_some_and(|rest| {
            let segments: Vec<&str> = rest.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
            !segments.is_empty() && segments.iter().all(|s| *s != "." && *s != "..")
        });
        if !below {
            return self.error(
                diagnostic::MANIFEST_VALUE,
                folder.at,
                format!(
                    "`{}` is a whole folder others use too; name the app's folder in it, like `{}/MyApp`",
                    folder.text, folder.text
                ),
            );
        }
        self.purge.push(folder.text.clone());
    }

    /// `expect <file> ["message"]`: the build fails unless the file is
    /// there, like the manifest a driver folder must hold.
    fn expect(&mut self, node: &Node, scope: &Scope) {
        self.no_children(node);
        let (file, message) = match node.args() {
            [file] => (file, None),
            [file, message] => (file, Some(message.text.clone())),
            _ => return self.syntax(node.at(), "expected `expect <file> [\"message\"]`"),
        };
        let source = self.reference(file).into_inner();
        let when = self.when(scope, &[]);
        let when = crate::manifest::selector::selector(
            when.as_ref(),
            self.file,
            self.source,
            &mut self.diagnostics,
        );
        self.expects.push(Expect {
            source,
            message,
            component: scope.component.as_ref().map(|c| c.get_ref().clone()),
            when,
            at: Location::at(self.file, self.source, node.at()),
        });
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
                // the setup's and the uninstall entry's own, apart from any app's.
                "icon" => self.icon_line(child, "package", &Scope::default()),
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

    /// `icon <os> <file>`, for the app or package called `owner`. an .ico is
    /// installed too: shortcuts and the uninstall entry show it.
    fn icon_line(&mut self, node: &Node, owner: &str, scope: &Scope) {
        let [os, path] = node.args() else {
            self.syntax(node.at(), "expected `icon <os> <file>`");
            return;
        };
        let when = self.when(scope, std::slice::from_ref(os));
        let source = self.reference(path);
        if path.text.to_ascii_lowercase().ends_with(".ico") {
            let file_when = self.when(scope, std::slice::from_ref(os));
            self.files.push(RawFile {
                logical_id: Some(spanned_text(path, format!("{owner}-icon"))),
                component: scope.component.clone(),
                source: source.clone(),
                target: spanned_text(path, format!("{{app}}/{}", basename(&path.text))),
                ownership: None,
                when: file_when,
            });
        }
        self.icons.push(RawIcon {
            source,
            size: None,
            when,
        });
    }

    fn app(&mut self, node: &Node, scope: &Scope) {
        let Some([id]) = self.args(node, 1, "app <id>") else {
            return;
        };
        let mut to = "{app}".to_owned();
        let mut variants = 0;
        // `to` applies to every variant, wherever it sits in the block
        for child in node.children.iter().filter(|c| c.keyword() == "to") {
            match child.args() {
                [dir] => dir.text.trim_end_matches('/').clone_into(&mut to),
                _ => self.syntax(child.at(), "expected `to <directory>`"),
            }
        }
        for child in &node.children {
            let args = child.args();
            match child.keyword() {
                "to" => {}
                "icon" => self.icon_line(child, &id.text, scope),
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
        let mut start = None;
        let mut restart = None;
        for child in &node.children {
            match child.keyword() {
                "name" => name = Some(rest(child.args())),
                "scope" => service_scope = self.one(child),
                "start" => start = self.one(child),
                "restart" => restart = self.one(child),
                other => self.syntax(
                    child.at(),
                    format!(
                        "unknown service field `{other}`; expected name, scope, start or restart"
                    ),
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
            start,
            restart,
            component: scope.component.clone(),
            when,
        });
    }

    /// `firewall <app> ["Rule name"] [public]`.
    fn firewall_rule(&mut self, node: &Node, scope: &Scope) {
        self.no_children(node);
        let usage = "expected `firewall <app> [\"Rule name\"] [public] [tcp|udp [ports]]`";
        let [app, rest @ ..] = node.args() else {
            return self.syntax(node.at(), usage);
        };
        let mut name = None;
        let mut public = false;
        let mut protocol = None;
        let mut ports = String::new();
        let mut words = rest.iter().peekable();
        while let Some(word) = words.next() {
            match word.text.as_str() {
                "public" if !word.quoted => public = true,
                "tcp" | "udp" if !word.quoted && protocol.is_none() => {
                    protocol = word.text.parse().ok();
                    // the ports, when the next word is a port list.
                    if let Some(next) = words.peek()
                        && !next.quoted
                        && next.text.starts_with(|c: char| c.is_ascii_digit())
                    {
                        if let Err(e) = crate::manifest::check_ports(&next.text) {
                            return self.error(diagnostic::MANIFEST_VALUE, next.at, e);
                        }
                        next.text.clone_into(&mut ports);
                        words.next();
                    }
                }
                _ if name.is_none() => name = Some(word.text.clone()),
                _ => return self.syntax(word.at, usage),
            }
        }
        let when = self.when(scope, &[]);
        let when = crate::manifest::selector::selector(
            when.as_ref(),
            self.file,
            self.source,
            &mut self.diagnostics,
        );
        self.firewall.push(FirewallRule {
            app: app.text.clone(),
            name,
            public,
            protocol,
            ports,
            component: scope.component.as_ref().map(|c| c.get_ref().clone()),
            when,
            at: Location::at(self.file, self.source, node.at()),
        });
    }

    /// `driver {app}/path/x.inf`, with `device <hardware id>` lines under
    /// it for the software devices to make.
    fn driver(&mut self, node: &Node, scope: &Scope) {
        let usage = "expected `driver {app}/<folder>/<name>.inf`, with `device <hardware id>` lines under it";
        let [inf] = node.args() else {
            return self.syntax(node.at(), usage);
        };
        let path = match inf.text.parse::<crate::manifest::SemanticPath>() {
            Ok(path) => path,
            Err(e) => {
                return self.error(
                    diagnostic::MANIFEST_VALUE,
                    inf.at,
                    format!("{e}; the driver is a file the install puts in place, like `{{app}}/drivers/x.inf`"),
                );
            }
        };
        let is_inf = path
            .segments()
            .last()
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".inf"));
        if !is_inf {
            return self.error(
                diagnostic::MANIFEST_VALUE,
                inf.at,
                format!("`{}` is not an `.inf`; `driver` takes the driver package's setup information file", inf.text),
            );
        }
        let mut devices = Vec::new();
        for child in &node.children {
            self.no_children(child);
            match (child.keyword(), child.args()) {
                ("device", [id]) => match crate::manifest::check_hardware_id(&id.text) {
                    Ok(()) if devices.contains(&id.text) => self.error(
                        diagnostic::MANIFEST_DUPLICATE,
                        id.at,
                        format!("device `{}` twice", id.text),
                    ),
                    Ok(()) => devices.push(id.text.clone()),
                    Err(e) => self.error(diagnostic::MANIFEST_VALUE, id.at, e),
                },
                _ => self.syntax(
                    child.at(),
                    r"expected `device <hardware id>`, like `device Root\MyDevice`",
                ),
            }
        }
        let when = self.when(scope, &[]);
        let when = crate::manifest::selector::selector(
            when.as_ref(),
            self.file,
            self.source,
            &mut self.diagnostics,
        );
        self.drivers.push(Driver {
            inf: path,
            devices,
            component: scope.component.as_ref().map(|c| c.get_ref().clone()),
            when,
            at: Location::at(self.file, self.source, node.at()),
        });
    }

    /// `json <file>`, `registry <key>` or `env`, with `set|add|init <key>
    /// <value>` on the same line or indented under it.
    fn change(&mut self, node: &Node, scope: &Scope) {
        let kind = node.keyword();
        let (place, edits) = match (kind, node.args()) {
            ("env", edits) => (None, edits),
            (_, [place, edits @ ..]) => (Some(place), edits),
            _ => {
                self.syntax(node.at(), change_usage(kind));
                return;
            }
        };
        let when = self.when(scope, &[]);
        let when = crate::manifest::selector::selector(
            when.as_ref(),
            self.file,
            self.source,
            &mut self.diagnostics,
        );
        let mut lines: Vec<&[Word]> = node.children.iter().map(|c| &c.words[..]).collect();
        if !edits.is_empty() {
            lines.insert(0, edits);
        }
        if lines.is_empty() {
            self.syntax(node.at(), change_usage(kind));
        }
        for line in lines {
            let [edit, key, value] = line else {
                self.syntax(line.first().map_or(node.at(), |w| w.at), change_usage(kind));
                continue;
            };
            let edit_kind = match edit.text.parse::<Edit>() {
                Ok(e) => e,
                Err(e) => {
                    self.error(diagnostic::MANIFEST_VALUE, edit.at, e);
                    continue;
                }
            };
            let Some(place) = self.place(kind, place, key) else {
                continue;
            };
            if edit_kind == Edit::Add && matches!(place, Place::Registry { .. }) {
                self.error(
                    diagnostic::MANIFEST_VALUE,
                    edit.at,
                    "a registry value is not a list; use `set` or `init`",
                );
                continue;
            }
            self.changes.push(Change {
                place,
                edit: edit_kind,
                value: Value::word(&value.text, value.quoted),
                component: scope.component.as_ref().map(|c| c.get_ref().clone()),
                when: when.clone(),
                at: Location::at(self.file, self.source, edit.at),
            });
        }
    }

    /// where a change goes: the file or registry key of its statement plus
    /// the key or name on its line.
    fn place(&mut self, kind: &str, place: Option<&Word>, key: &Word) -> Option<Place> {
        match (kind, place) {
            ("json", Some(file)) => {
                if let Err(e) = location(&file.text) {
                    self.error(diagnostic::MANIFEST_VALUE, file.at, e);
                    return None;
                }
                let names: Vec<String> = key.text.split('.').map(str::to_owned).collect();
                if names.iter().any(String::is_empty) {
                    self.error(
                        diagnostic::MANIFEST_VALUE,
                        key.at,
                        format!("`{}` is not a key path like `a.b.c`", key.text),
                    );
                    return None;
                }
                Some(Place::Json {
                    file: file.text.clone(),
                    key: names,
                })
            }
            ("registry", Some(registry)) => {
                let path = registry.text.trim_matches('\\');
                if path.is_empty() || path.contains("\\\\") {
                    self.error(
                        diagnostic::MANIFEST_VALUE,
                        registry.at,
                        format!("`{}` is not a registry key", registry.text),
                    );
                    return None;
                }
                if path.contains('/') {
                    self.error(
                        diagnostic::MANIFEST_VALUE,
                        registry.at,
                        format!(
                            "`{}` has a `/` in the registry key; separate keys with `\\`",
                            registry.text
                        ),
                    );
                    return None;
                }
                let hive = path.split('\\').next().unwrap_or_default();
                if hive.len() >= 2
                    && hive[..2].eq_ignore_ascii_case("HK")
                    && !["HKCU", "HKEY_CURRENT_USER", "HKLM", "HKEY_LOCAL_MACHINE"]
                        .iter()
                        .any(|h| hive.eq_ignore_ascii_case(h))
                {
                    self.diagnostics.push(
                        Diagnostic::error(
                            diagnostic::MANIFEST_VALUE,
                            format!("`{hive}` is not a supported registry hive"),
                        )
                        .at(Location::at(self.file, self.source, registry.at))
                        .help("use HKCU or HKLM; for file types and protocols write `HKCU\\Software\\Classes\\...`"),
                    );
                    return None;
                }
                Some(Place::Registry {
                    key: path.to_owned(),
                    name: if key.text == "@" {
                        String::new()
                    } else {
                        key.text.clone()
                    },
                })
            }
            _ => {
                let valid = !key.text.is_empty()
                    && key
                        .text
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !valid {
                    self.error(
                        diagnostic::MANIFEST_VALUE,
                        key.at,
                        format!("`{}` is not an environment variable name", key.text),
                    );
                    return None;
                }
                Some(Place::Environment {
                    name: key.text.clone(),
                })
            }
        }
    }

    /// `check <id> ["Name"] <test>`, the test on the same line or indented
    /// under it: `registry <key> [value]`, `file <path>` or `json <file>
    /// <key.path>`.
    fn check_block(&mut self, node: &Node) {
        let usage = "expected `check <id> [\"Name\"]` and `registry <key> [value]`, `file <path>` or `json <file> <key.path>`";
        let [id, rest_words @ ..] = node.args() else {
            self.syntax(node.at(), usage);
            return;
        };
        let (name, inline) = match rest_words {
            [name, rest @ ..] if name.quoted => (name.text.clone(), rest),
            rest => (id.text.clone(), rest),
        };
        let test: &[Word] = match (inline, node.children.as_slice()) {
            ([_, ..], []) => inline,
            ([], [child]) => &child.words,
            _ => {
                self.syntax(node.at(), usage);
                return;
            }
        };
        let Some(check) = self.test(test, node.at()) else {
            return;
        };
        if self.checks.contains_key(&id.text) && !self.library_checks.remove(&id.text) {
            self.error(
                diagnostic::MANIFEST_DUPLICATE,
                id.at,
                format!("check `{}` is declared twice", id.text),
            );
            return;
        }
        self.checks.insert(id.text.clone(), Detect { name, check });
    }

    /// a test written out, or the id of a `check`.
    fn detect(&mut self, words: &[Word], at: usize) -> Option<Detect> {
        if let [id] = words
            && !matches!(id.text.as_str(), "registry" | "file" | "json")
        {
            let found = self.checks.get(&id.text).cloned();
            if found.is_none() {
                self.error(
                    diagnostic::MANIFEST_REFERENCE,
                    id.at,
                    format!(
                        "no check `{}`; define one with `check {} \"Name\"` and a test under it",
                        id.text, id.text
                    ),
                );
            }
            return found;
        }
        let check = self.test(words, at)?;
        let name = words.get(1).map_or_else(String::new, |w| w.text.clone());
        Some(Detect { name, check })
    }

    /// `registry <key> [value]`, `file <path>` or `json <file> <key.path>`.
    fn test(&mut self, words: &[Word], at: usize) -> Option<Check> {
        let texts: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        match texts.as_slice() {
            ["registry", key] => Some(Check::Registry {
                key: (*key).to_owned(),
                value: None,
            }),
            ["registry", key, value] => Some(Check::Registry {
                key: (*key).to_owned(),
                value: Some((*value).to_owned()),
            }),
            ["file", _, ..] => Some(Check::File {
                path: rest(&words[1..]),
            }),
            ["json", file, key] => Some(Check::Json {
                file: (*file).to_owned(),
                key: key.split('.').map(str::to_owned).collect(),
            }),
            _ => {
                self.syntax(
                    words.first().map_or(at, |w| w.at),
                    "expected `registry <key> [value]`, `file <path>` or `json <file> <key.path>`",
                );
                None
            }
        }
    }

    /// `run <program> [arguments...]`, with `at install|uninstall`,
    /// `undo <program> [arguments...]` and, at uninstall, `required` under
    /// it.
    fn run(&mut self, node: &Node, scope: &Scope) {
        let Some(command) = invocation(node.args()) else {
            return self.syntax(node.at(), "expected `run <program> [arguments...]`");
        };
        let mut phase = RunAt::Install;
        let mut undo = None;
        let mut required = false;
        for child in &node.children {
            self.no_children(child);
            match (child.keyword(), child.args()) {
                ("required", []) => required = true,
                ("at", [when]) => match when.text.parse() {
                    Ok(p) => phase = p,
                    Err(e) => self.error(diagnostic::MANIFEST_VALUE, when.at, e),
                },
                ("undo", args) if !args.is_empty() => undo = invocation(args),
                _ => self.syntax(
                    child.at(),
                    "expected `at install|uninstall`, `undo <program> [arguments...]` or `required`",
                ),
            }
        }
        if phase == RunAt::Uninstall && undo.is_some() {
            return self.syntax(node.at(), "a program run at uninstall has nothing to undo");
        }
        if required && phase != RunAt::Uninstall {
            return self.syntax(
                node.at(),
                "`required` is for a program run at uninstall; a failing one at install already undoes the install",
            );
        }
        let when = self.when(scope, &[]);
        let when = crate::manifest::selector::selector(
            when.as_ref(),
            self.file,
            self.source,
            &mut self.diagnostics,
        );
        self.runs.push(Run {
            phase,
            command,
            undo,
            required,
            component: scope.component.as_ref().map(|c| c.get_ref().clone()),
            when,
            at: Location::at(self.file, self.source, node.at()),
        });
    }

    /// `require <os> <minimum>` or `require <software>`, with
    /// `install-if-missing` to ask for more than a check.
    fn require(&mut self, node: &Node, scope: &Scope) {
        self.no_children(node);
        let usage = "expected `require windows 10.0.19041` or `require <check>`";
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
        let detect = if is_os {
            None
        } else {
            let Some(d) = self.detect(std::slice::from_ref(*what), what.at) else {
                return;
            };
            Some(d)
        };
        self.requirements.push(RawRequirement {
            id: spanned(what),
            kind: spanned_text(what, kind.to_owned()),
            policy,
            version: None,
            minimum,
            detect,
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
                    detect = self.detect(child.args(), child.at());
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
                "banner" => self.brand.banner = image(self),
                "link" => match args {
                    [label, url] => {
                        self.brand.link = Some((label.text.clone(), url.text.clone()));
                    }
                    _ => self.syntax(child.at(), "expected `link \"Label\" <url>`"),
                },
                other => self.syntax(
                    child.at(),
                    format!(
                        "unknown brand field `{other}`; expected text, accent, logo, banner, header, side, welcome, finish or link"
                    ),
                ),
            }
        }
    }

    /// `sign` with one key line (`certificate`, `pfx` or `azure`), and
    /// `timestamp` and `programs off` when wanted.
    fn sign_block(&mut self, node: &Node) {
        if !node.args().is_empty() {
            self.syntax(node.at(), "`sign` takes its settings on the lines under it");
        }
        let mut key = None;
        let mut timestamp = TIMESTAMP.to_owned();
        let mut programs = true;
        for child in &node.children {
            self.no_children(child);
            let args = child.args();
            let words: Vec<&str> = args.iter().map(|w| w.text.as_str()).collect();
            let found = match (child.keyword(), words.as_slice()) {
                ("certificate", ["machine", rest @ ..]) => {
                    setting(rest).map(|t| SignKey::Certificate {
                        thumbprint: t,
                        machine: true,
                    })
                }
                ("certificate", rest) => setting(rest).map(|t| SignKey::Certificate {
                    thumbprint: t,
                    machine: false,
                }),
                ("pfx", [_]) => Some(SignKey::Pfx {
                    file: self.reference(&args[0]).into_inner(),
                    password: None,
                }),
                ("pfx", [_, "env", name]) => Some(SignKey::Pfx {
                    file: self.reference(&args[0]).into_inner(),
                    password: Some((*name).to_owned()),
                }),
                ("azure", [_, "dlib", _]) => Some(SignKey::Azure {
                    metadata: self.reference(&args[0]).into_inner(),
                    dlib: args[2].text.clone(),
                }),
                ("timestamp", [url]) => {
                    (*url).clone_into(&mut timestamp);
                    continue;
                }
                ("programs", ["on" | "off"]) => {
                    programs = words[0] == "on";
                    continue;
                }
                ("pfx" | "azure" | "timestamp" | "programs", _) => {
                    self.syntax(child.at(), sign_usage(child.keyword()));
                    continue;
                }
                (other, _) => {
                    self.syntax(
                        child.at(),
                        format!(
                            "unknown sign field `{other}`; expected certificate, pfx, azure, timestamp or programs"
                        ),
                    );
                    continue;
                }
            };
            match found {
                Some(_) if key.is_some() => {
                    self.syntax(child.at(), "`sign` takes one of certificate, pfx or azure");
                }
                Some(found) => key = Some(found),
                None => self.syntax(child.at(), sign_usage(child.keyword())),
            }
        }
        match key {
            Some(key) => {
                self.sign = Some(Sign {
                    key,
                    timestamp,
                    programs,
                });
            }
            None => self.syntax(
                node.at(),
                "`sign` needs where the key is: `certificate <thumbprint>`, `pfx <file> env <VAR>` or `azure <metadata.json> dlib <path>`",
            ),
        }
    }

    /// `prerequisite <id> ["Name"]` with `file`, and `arguments`, `detect`
    /// and `admin` unless kago knows the id.
    fn prerequisite(&mut self, node: &Node) {
        let (id, name) = match node.args() {
            [id] => (id, None),
            [id, name] => (id, Some(name.text.clone())),
            _ => {
                self.syntax(node.at(), "expected `prerequisite <id> [\"Name\"]`");
                return;
            }
        };
        let mut file = None;
        let mut arguments = Vec::new();
        let mut check = None;
        let mut admin = false;
        for child in &node.children {
            self.no_children(child);
            let args = child.args();
            let words: Vec<&str> = args.iter().map(|w| w.text.as_str()).collect();
            match (child.keyword(), words.as_slice()) {
                ("file", [_]) => file = Some(self.reference(&args[0]).into_inner()),
                ("arguments", _) => arguments = words.iter().map(|w| (*w).to_owned()).collect(),
                ("admin", []) => admin = true,
                ("detect", _) => {
                    if let Some(d) = self.detect(args, child.at()) {
                        check = Some(d.check);
                    }
                }
                ("file" | "admin", _) => {
                    self.syntax(child.at(), "expected `file <installer>` or `admin`");
                }
                (other, _) => self.syntax(
                    child.at(),
                    format!(
                        "unknown prerequisite field `{other}`; expected file, arguments, detect or admin"
                    ),
                ),
            }
        }
        let Some(file) = file else {
            self.syntax(
                node.at(),
                "a prerequisite needs `file <installer>`, like `file vcredist:vc_redist.x64.exe` from a pinned `source`",
            );
            return;
        };
        let Some(check) = check else {
            self.syntax(
                node.at(),
                "a prerequisite needs `detect ...` to tell whether it is installed already; kago's library has `vcredist <file>` and `webview2 <file>` ready",
            );
            return;
        };
        self.prerequisites.push(Prerequisite {
            name: name.unwrap_or_else(|| id.text.clone()),
            id: id.text.clone(),
            file,
            arguments,
            check,
            admin,
            at: Location::at(self.file, self.source, node.at()),
        });
    }

    /// `update github <owner>/<repo>` or `update feed <https url>`.
    fn update(&mut self, node: &Node) {
        self.no_children(node);
        let feed = match node.args() {
            [kind, repo] if kind.text == "github" => {
                let parts: Vec<&str> = repo.text.split('/').collect();
                if parts.len() != 2 || parts.iter().any(|p| p.is_empty()) {
                    self.error(
                        diagnostic::MANIFEST_VALUE,
                        repo.at,
                        "expected the repository as `owner/name`",
                    );
                    return;
                }
                spanned_text(repo, crate::manifest::Update::github(&repo.text).feed)
            }
            [kind, url] if kind.text == "feed" => {
                if !url.text.starts_with("https://") && !url.text.starts_with("file://") {
                    self.error(
                        diagnostic::MANIFEST_VALUE,
                        url.at,
                        "an update feed has to be https, so nobody can swap it on the way",
                    );
                    return;
                }
                spanned(url)
            }
            _ => {
                self.syntax(
                    node.at(),
                    "expected `update github <owner>/<repo>` or `update feed <https url>`",
                );
                return;
            }
        };
        if self.update.is_some() {
            self.syntax(node.at(), "only one `update` line");
        }
        self.update = Some(RawUpdate { feed });
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
            shortcuts: self.shortcuts,
            services: self.services,
            requirements: self.requirements,
            update: self.update,
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
            sign: self.sign,
            prerequisites: self.prerequisites,
            firewall: self.firewall,
            drivers: self.drivers,
            changes: self.changes,
            expects: self.expects,
            runs: self.runs,
            build: self.build,
            inputs: self.inputs,
            purge: self.purge,
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

fn change_usage(kind: &str) -> &'static str {
    match kind {
        "json" => "expected `json <file>` with `set|add|init <key.path> <value>` lines",
        "registry" => {
            "expected `registry <key>` with `set|init <name> <value>` lines; `@` names the default value"
        }
        _ => "expected `env` with `set|add|init <NAME> <value>` lines",
    }
}

/// checks a place outside the package written as `{root}/...` or
/// `%VARIABLE%/...`. kago fills both in at install.
fn location(text: &str) -> Result<(), String> {
    if text.starts_with('{') {
        return text
            .parse::<crate::manifest::SemanticPath>()
            .map(|_| ())
            .map_err(|e| e.to_string());
    }
    let named = text
        .strip_prefix('%')
        .and_then(|rest| rest.split_once('%'))
        .is_some_and(|(name, rest)| {
            !name.is_empty() && (rest.is_empty() || rest.starts_with(['/', '\\']))
        });
    if named && !text.split(['/', '\\']).any(|s| s == "..") {
        Ok(())
    } else {
        Err(format!(
            "`{text}` must start with a root like `{{config}}/` or a variable like `%LOCALAPPDATA%/`"
        ))
    }
}

/// `<program> [arguments...]`.
fn invocation(words: &[Word]) -> Option<Command> {
    let (program, arguments) = words.split_first()?;
    Some(Command {
        program: program.text.clone(),
        arguments: arguments.iter().map(|w| w.text.clone()).collect(),
    })
}
