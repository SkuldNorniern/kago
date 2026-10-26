//! which page the setup is on and what each button does there. both looks
//! drive the same wizard, so the flow is the same whichever one draws it.

use std::path::{Path, PathBuf};

use super::{Component, Existing, Installed, Place, Step};
use crate::install::LocationError;
use crate::manifest::{Field, InputPage, Problem};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Welcome,
    License,
    /// for everyone or just this user.
    Scope,
    Components,
    Directory,
    /// one of the package's own pages, by index.
    Input(usize),
    /// programs use the installed files; installing closes them.
    Running,
    Installing,
    Done,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// next, install or finish, whichever the page has.
    Next,
    Back,
    /// accept the license and go on, like NSIS's "I Agree".
    Agree,
    /// install now when the license allows, else show the license.
    Install,
    ToggleAccept,
    /// install for everyone (true) or just this user.
    SetEveryone(bool),
    /// tick or untick the optional component at this index.
    SetComponent(usize, bool),
    /// run the app after finishing.
    SetOpen(bool),
    /// open the folder dialog; the window handles it.
    Browse,
    /// open the brand link; the window handles it.
    OpenLink,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    /// the install should start now.
    Start,
    /// close the window, then run `launch` if there is one.
    Exit {
        launch: Option<PathBuf>,
    },
}

/// an optional component as the wizard tracks it.
#[derive(Debug, Clone)]
struct Offer {
    id: String,
    depends: Vec<String>,
    required: bool,
    size: u64,
}

#[derive(Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent answers the pages collect"
)]
pub struct Wizard {
    pub page: Page,
    pub has_license: bool,
    pub must_accept: bool,
    pub accepted: bool,
    /// ticked state of each optional component, in offer order.
    pub chosen: Vec<bool>,
    offers: Vec<Offer>,
    /// where `{app}` goes.
    pub location: Option<PathBuf>,
    /// what is installed already where `location` is.
    pub existing: Existing,
    /// the person may change `location`.
    pub folder_open: bool,
    /// the window can pick folders at all.
    folder: bool,
    /// installing for everyone rather than just this user.
    pub for_everyone: bool,
    /// the other way to install, when the person may pick it.
    other: Option<Place>,
    /// why installing did not start, like a declined administrator prompt.
    pub notice: Option<String>,
    /// the page installing started from, to go back to when it cannot.
    before: Page,
    /// programs using the installed files, shown on the running page.
    pub running: Vec<String>,
    /// the person agreed to close them.
    pub close_ok: bool,
    /// bytes installed whatever is ticked.
    pub base_size: u64,
    /// bytes free on the drive of `location`, when known.
    pub free: Option<u64>,
    /// why `location` cannot be used, if it cannot.
    pub folder_error: Option<LocationError>,
    /// a newer version is installed that this one may not replace.
    pub blocked: bool,
    /// 0 to 1.
    pub fraction: f32,
    pub log: Vec<Step>,
    pub launch: Option<PathBuf>,
    pub open_after: bool,
    pub error: Option<String>,
    /// the package's own pages.
    pub inputs: Vec<InputPage>,
    /// the answer to every field of `inputs`, in page order.
    pub answers: Vec<String>,
    /// the text field typing goes into.
    pub focus: Option<usize>,
    /// a field whose answer does not fit, and why.
    pub problem: Option<(usize, Problem)>,
    /// the modern look: one welcome page picks everything else, so only
    /// the license and the package's own pages follow it.
    pub compact: bool,
    /// updating an install: secret answers were not kept and are asked again.
    pub updating: bool,
}

impl Wizard {
    #[must_use]
    pub fn new(
        license: Option<bool>,
        components: &[Component],
        place: Place,
        folder: bool,
    ) -> Self {
        let mut wizard = Self {
            page: Page::Welcome,
            has_license: license.is_some(),
            must_accept: license.unwrap_or(false),
            accepted: false,
            chosen: components.iter().map(|c| c.default || c.required).collect(),
            offers: components
                .iter()
                .map(|c| Offer {
                    id: c.id.clone(),
                    depends: c.depends.clone(),
                    required: c.required,
                    size: c.size,
                })
                .collect(),
            folder_open: folder && !place.existing.installed(),
            blocked: place.existing.blocks(),
            location: place.location,
            existing: place.existing,
            folder,
            for_everyone: false,
            other: None,
            notice: None,
            before: Page::Welcome,
            running: Vec::new(),
            close_ok: false,
            base_size: 0,
            free: None,
            folder_error: None,
            fraction: 0.0,
            log: Vec::new(),
            launch: None,
            open_after: true,
            error: None,
            inputs: Vec::new(),
            answers: Vec::new(),
            focus: None,
            problem: None,
            compact: false,
            updating: false,
        };
        for i in 0..wizard.chosen.len() {
            if wizard.locked(i) {
                wizard.chosen[i] = true;
            }
        }
        wizard
    }

    /// lets the person pick between installing for everyone and just for
    /// them; `other` is where the other way goes.
    pub fn offer_scope(&mut self, for_everyone: bool, other: Place) {
        self.for_everyone = for_everyone;
        self.other = Some(other);
    }

    /// the package's own pages, each field starting at `earlier`'s answer
    /// when it fits, else its default.
    pub fn ask(
        &mut self,
        inputs: Vec<InputPage>,
        earlier: &std::collections::BTreeMap<String, String>,
    ) {
        self.answers = inputs
            .iter()
            .flat_map(|p| &p.fields)
            .map(|f| {
                earlier
                    .get(&f.id)
                    .filter(|a| !f.secret() && f.check(a).is_ok())
                    .cloned()
                    .unwrap_or_else(|| f.default.clone())
            })
            .collect();
        self.inputs = inputs;
    }

    /// every field with its index into `answers`, page by page.
    pub fn fields(&self) -> impl Iterator<Item = (usize, &Field)> {
        self.inputs.iter().flat_map(|p| &p.fields).enumerate()
    }

    /// the fields of page `page`, with their indexes into `answers`.
    #[must_use]
    pub fn page_fields(&self, page: usize) -> Vec<(usize, &Field)> {
        let start: usize = self.inputs.iter().take(page).map(|p| p.fields.len()).sum();
        self.inputs
            .get(page)
            .map(|p| {
                p.fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| (start + i, f))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// the answers by field id, for the install.
    #[must_use]
    pub fn answers_by_id(&self) -> std::collections::BTreeMap<String, String> {
        self.fields()
            .map(|(i, f)| (f.id.clone(), self.answers[i].clone()))
            .collect()
    }

    /// sets the answer of field `index`, like a picked option.
    pub fn set_answer(&mut self, index: usize, answer: String) {
        if let Some(a) = self.answers.get_mut(index) {
            *a = answer;
            if self.problem.is_some_and(|(i, _)| i == index) {
                self.problem = None;
            }
        }
    }

    /// typing goes into field `index`, when it is one to type in.
    pub fn focus(&mut self, index: usize) {
        if self.typed(index) {
            self.focus = Some(index);
        }
    }

    /// whether field `index` takes typed text, unlike a choice or a tick.
    fn typed(&self, index: usize) -> bool {
        self.fields().any(|(at, f)| {
            at == index
                && matches!(
                    f.kind,
                    crate::manifest::FieldKind::Text { .. }
                        | crate::manifest::FieldKind::Number { .. }
                )
        })
    }

    /// typed text, into the focused field.
    pub fn type_text(&mut self, text: &str) {
        if let Some(i) = self.focus {
            let mut answer = self.answers.get(i).cloned().unwrap_or_default();
            answer.push_str(text);
            self.set_answer(i, answer);
        }
    }

    /// pasted text, into the focused field: its first line, without
    /// control characters, since a field holds one line.
    pub fn paste(&mut self, text: &str) {
        let line: String = text
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        self.type_text(&line);
    }

    /// what the focused field holds, to copy; never a secret one.
    #[must_use]
    pub fn copied(&self) -> Option<&str> {
        let i = self.focus?;
        let (_, field) = self.fields().find(|(at, _)| *at == i)?;
        (!field.secret()).then(|| self.answers[i].as_str())
    }

    /// empties the focused field, like Ctrl+Backspace.
    pub fn clear(&mut self) {
        if let Some(i) = self.focus {
            self.set_answer(i, String::new());
        }
    }

    /// Backspace in the focused field.
    pub fn erase(&mut self) {
        if let Some(i) = self.focus {
            let mut answer = self.answers.get(i).cloned().unwrap_or_default();
            answer.pop();
            self.set_answer(i, answer);
        }
    }

    /// the first field on input page `page` whose answer does not fit.
    fn page_problem(&self, page: usize) -> Option<(usize, Problem)> {
        self.page_fields(page).into_iter().find_map(|(i, f)| {
            let answer = &self.answers[i];
            if self.updating && f.secret() && answer.is_empty() {
                return Some((i, Problem::Again));
            }
            f.check(answer).err().map(|p| (i, p))
        })
    }

    /// whether the person may pick between everyone and just them.
    #[must_use]
    pub const fn scope_open(&self) -> bool {
        self.other.is_some()
    }

    /// switches to installing for everyone or just this user, with that
    /// way's folder and what is installed there.
    fn set_everyone(&mut self, on: bool) {
        if on == self.for_everyone {
            return;
        }
        let Some(other) = self.other.take() else {
            return;
        };
        let current = Place {
            location: self.location.take(),
            existing: std::mem::take(&mut self.existing),
        };
        self.for_everyone = on;
        self.folder_open = self.folder && !other.existing.installed();
        self.blocked = other.existing.blocks();
        self.location = other.location;
        self.existing = other.existing;
        self.folder_error = None;
        self.notice = None;
        self.other = Some(current);
    }

    /// whether Next works on this page right now.
    #[must_use]
    pub fn can_go_on(&self) -> bool {
        match self.page {
            Page::License => self.accepted || !self.must_accept,
            Page::Installing => false,
            _ => true,
        }
    }

    /// a folder typed or picked, with what checking it found.
    pub fn set_location(&mut self, dir: &Path, check: Result<(), LocationError>) {
        if self.folder_open {
            self.location = Some(dir.to_path_buf());
            self.folder_error = check.err();
        }
    }

    /// the folder to install into when the person chose one.
    #[must_use]
    pub fn chosen_location(&self) -> Option<PathBuf> {
        self.location.clone().filter(|_| self.folder_open)
    }

    /// ids of the ticked components, for the install.
    #[must_use]
    pub fn chosen_ids(&self) -> Vec<String> {
        self.offers
            .iter()
            .zip(&self.chosen)
            .filter(|(_, on)| **on)
            .map(|(o, _)| o.id.clone())
            .collect()
    }

    /// the pages before installing, in order. which there are depends on
    /// the package and, for the folder, on what is installed already.
    fn pages(&self) -> Vec<Page> {
        let inputs = (0..self.inputs.len()).map(Page::Input);
        if self.compact {
            return std::iter::once(Page::Welcome).chain(inputs).collect();
        }
        let mut pages = vec![Page::Welcome];
        if self.has_license {
            pages.push(Page::License);
        }
        if self.scope_open() {
            pages.push(Page::Scope);
        }
        if !self.offers.is_empty() {
            pages.push(Page::Components);
        }
        if self.folder_open {
            pages.push(Page::Directory);
        }
        pages.extend(inputs);
        pages
    }

    /// the package's own pages when there are any, else installing: what
    /// comes after the modern look's welcome page and license.
    fn after_choices(&mut self) -> Outcome {
        if self.inputs.is_empty() {
            self.start()
        } else {
            self.show(Page::Input(0))
        }
    }

    /// whether Next on this page starts installing.
    #[must_use]
    pub fn installs_next(&self) -> bool {
        if self.compact && self.page == Page::License {
            return self.inputs.is_empty();
        }
        self.pages().last() == Some(&self.page)
    }

    /// the page after this one, or installing after the last.
    fn next(&mut self) -> Outcome {
        if let Page::Input(page) = self.page
            && let Some(problem) = self.page_problem(page)
        {
            self.problem = Some(problem);
            self.focus = self.typed(problem.0).then_some(problem.0);
            return Outcome::Stay;
        }
        self.problem = None;
        if self.compact && matches!(self.page, Page::Welcome | Page::License) {
            return self.after_choices();
        }
        let pages = self.pages();
        let at = pages.iter().position(|p| *p == self.page);
        match at.and_then(|i| pages.get(i + 1)) {
            Some(page) => self.show(*page),
            None => self.start(),
        }
    }

    fn back(&mut self) -> Outcome {
        self.problem = None;
        self.focus = None;
        let pages = self.pages();
        if let Some(i) = pages.iter().position(|p| *p == self.page)
            && i > 0
        {
            self.show(pages[i - 1]);
        }
        Outcome::Stay
    }

    /// whether the page is one where the person still decides things.
    const fn choosing(&self) -> bool {
        matches!(
            self.page,
            Page::Welcome
                | Page::License
                | Page::Scope
                | Page::Components
                | Page::Directory
                | Page::Input(_)
        )
    }

    pub fn act(&mut self, action: Action) -> Outcome {
        let choosing = self.choosing();
        match (self.page, action) {
            (Page::Installing, _) => Outcome::Stay,
            (Page::License, Action::Agree) => {
                self.accepted = true;
                self.next()
            }
            (Page::License, Action::Next) if !self.can_go_on() => Outcome::Stay,
            (Page::Running, Action::Next) => {
                self.close_ok = true;
                self.start()
            }
            (Page::Running, Action::Back) => self.show(self.before),
            (_, Action::Next) if choosing => self.next(),
            (_, Action::Back) if choosing => self.back(),
            (Page::Welcome | Page::License, Action::Install) => {
                if self.accepted || !self.must_accept {
                    self.after_choices()
                } else {
                    self.show(Page::License)
                }
            }
            (Page::Welcome | Page::License, Action::ToggleAccept) => {
                self.accepted = !self.accepted;
                Outcome::Stay
            }
            (_, Action::SetEveryone(on)) if choosing => {
                self.set_everyone(on);
                Outcome::Stay
            }
            (_, Action::SetComponent(i, on)) if choosing => {
                self.set_component(i, on);
                Outcome::Stay
            }
            (Page::Done, Action::SetOpen(open)) => {
                self.open_after = open;
                Outcome::Stay
            }
            (Page::Done, Action::Next) => Outcome::Exit {
                launch: self.launch.clone().filter(|_| self.open_after),
            },
            (_, Action::Cancel) | (Page::Failed, Action::Next) => Outcome::Exit { launch: None },
            _ => Outcome::Stay,
        }
    }

    /// ticking a component ticks what it needs; unticking one unticks what
    /// needs it.
    /// whether the component at `index` cannot be unticked: it is
    /// required, or something locked needs it.
    #[must_use]
    pub fn locked(&self, index: usize) -> bool {
        let Some(offer) = self.offers.get(index) else {
            return false;
        };
        // walk from the required ones down what they need.
        let mut seen: Vec<&str> = Vec::new();
        let mut todo: Vec<&str> = self
            .offers
            .iter()
            .filter(|o| o.required)
            .map(|o| o.id.as_str())
            .collect();
        while let Some(id) = todo.pop() {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            if let Some(o) = self.offers.iter().find(|o| o.id == id) {
                todo.extend(o.depends.iter().map(String::as_str));
            }
        }
        seen.contains(&offer.id.as_str())
    }

    fn set_component(&mut self, index: usize, on: bool) {
        let Some(offer) = self.offers.get(index) else {
            return;
        };
        if !on && self.locked(index) {
            return;
        }
        let mut todo = vec![offer.id.clone()];
        while let Some(id) = todo.pop() {
            let Some(i) = self.offers.iter().position(|o| o.id == id) else {
                continue;
            };
            if self.chosen[i] == on && id != self.offers[index].id {
                continue;
            }
            self.chosen[i] = on;
            if on {
                todo.extend(self.offers[i].depends.iter().cloned());
            } else {
                todo.extend(
                    self.offers
                        .iter()
                        .filter(|o| o.depends.contains(&id))
                        .map(|o| o.id.clone()),
                );
            }
        }
    }

    fn show(&mut self, page: Page) -> Outcome {
        self.page = page;
        // typing goes into the first text field of the page right away.
        if let Page::Input(index) = page {
            self.focus = self
                .page_fields(index)
                .into_iter()
                .find(|(_, f)| {
                    matches!(
                        f.kind,
                        crate::manifest::FieldKind::Text { .. }
                            | crate::manifest::FieldKind::Number { .. }
                    )
                })
                .map(|(i, _)| i);
        }
        Outcome::Stay
    }

    /// bytes the install takes with what is ticked.
    #[must_use]
    pub fn space_needed(&self) -> u64 {
        self.base_size
            + self
                .offers
                .iter()
                .zip(&self.chosen)
                .filter(|(_, on)| **on)
                .map(|(o, _)| o.size)
                .sum::<u64>()
    }

    /// whether the drive of `location` lacks room for what is ticked.
    #[must_use]
    pub fn short_of_space(&self) -> bool {
        self.free.is_some_and(|free| free < self.space_needed())
    }

    /// installing begins unless the folder is unusable.
    fn start(&mut self) -> Outcome {
        if self.folder_error.is_some() || self.blocked || self.short_of_space() {
            return Outcome::Stay;
        }
        if self.page != Page::Running {
            self.before = self.page;
        }
        self.notice = None;
        self.page = Page::Installing;
        Outcome::Start
    }

    /// installs with what is picked, skipping the pages left.
    pub fn install_now(&mut self) -> Outcome {
        self.start()
    }

    /// ticks exactly the `components` given, besides the locked ones.
    pub fn pick(&mut self, components: &[String]) {
        for i in 0..self.offers.len() {
            self.chosen[i] = self.locked(i) || components.contains(&self.offers[i].id);
        }
    }

    /// installing has to close `programs` first: asks on the running page.
    pub fn must_close(&mut self, programs: Vec<String>) {
        self.running = programs;
        self.page = Page::Running;
    }

    /// installing could not start after all: back where it started from,
    /// saying why.
    pub fn not_started(&mut self, why: String) {
        self.page = self.before;
        self.notice = Some(why);
    }

    pub fn progress(&mut self, fraction: f32, step: Step) {
        self.fraction = fraction.clamp(self.fraction, 1.0);
        self.log.push(step);
    }

    pub fn finished(&mut self, result: Result<Installed, String>) {
        match result {
            Ok(done) => {
                self.fraction = 1.0;
                self.open_after = done.launch.is_some();
                self.launch = done.launch;
                self.page = Page::Done;
            }
            Err(e) => {
                self.error = Some(e);
                self.page = Page::Failed;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(license: Option<bool>) -> Wizard {
        Wizard::new(license, &[], Place::default(), false)
    }

    fn at(dir: &Path) -> Place {
        Place {
            location: Some(dir.to_path_buf()),
            existing: Existing::None,
        }
    }

    fn component(id: &str, default: bool, depends: &[&str]) -> Component {
        Component {
            id: id.into(),
            name: id.into(),
            description: None,
            default,
            depends: depends.iter().map(|d| (*d).to_owned()).collect(),
            size: 0,
            missing: None,
            required: false,
        }
    }

    fn asking() -> Wizard {
        let mut w = plain(None);
        let page = |id: &str, field: Field| InputPage {
            id: id.into(),
            title: id.into(),
            about: None,
            fields: vec![field],
            skip: None,
        };
        let text = |id: &str, required: bool| Field {
            id: id.into(),
            label: id.into(),
            kind: crate::manifest::FieldKind::Text { secret: false },
            default: String::new(),
            required,
        };
        let earlier = std::collections::BTreeMap::from([("b".to_owned(), "kept".to_owned())]);
        w.ask(
            vec![page("one", text("a", true)), page("two", text("b", false))],
            &earlier,
        );
        w
    }

    #[test]
    fn an_update_asks_for_a_secret_again() {
        let mut w = plain(None);
        w.compact = true;
        w.updating = true;
        let token = Field {
            id: "token".into(),
            label: "token".into(),
            kind: crate::manifest::FieldKind::Text { secret: true },
            default: String::new(),
            required: false,
        };
        w.ask(
            vec![InputPage {
                id: "p".into(),
                title: "p".into(),
                about: None,
                fields: vec![token],
                skip: None,
            }],
            &std::collections::BTreeMap::new(),
        );
        w.act(Action::Install);
        assert_eq!(w.act(Action::Next), Outcome::Stay);
        assert_eq!(w.problem, Some((0, Problem::Again)));
        w.type_text("s3cret");
        assert_ne!(w.act(Action::Next), Outcome::Stay);
    }

    #[test]
    fn typing_never_goes_into_a_choice() {
        let mut w = plain(None);
        let choice = Field {
            id: "mode".into(),
            label: "mode".into(),
            kind: crate::manifest::FieldKind::Choice {
                options: vec!["local".into(), "cloud".into()],
            },
            default: "local".into(),
            required: true,
        };
        w.ask(
            vec![InputPage {
                id: "p".into(),
                title: "p".into(),
                about: None,
                fields: vec![choice],
                skip: None,
            }],
            &std::collections::BTreeMap::new(),
        );
        w.focus(0);
        w.type_text("x");
        w.erase();
        assert_eq!(w.focus, None);
        assert_eq!(w.answers, ["local"]);
    }

    #[test]
    fn the_modern_look_asks_its_pages_after_welcome() {
        let mut w = asking();
        w.compact = true;
        assert_eq!(
            w.answers,
            ["", "kept"],
            "an update starts from the earlier answers"
        );
        assert_eq!(w.act(Action::Install), Outcome::Stay);
        assert_eq!(w.page, Page::Input(0));
        assert!(!w.installs_next());
        // required and empty: stays, says why, typing goes there.
        assert_eq!(w.act(Action::Next), Outcome::Stay);
        assert_eq!(w.page, Page::Input(0));
        assert_eq!(w.problem, Some((0, Problem::Empty)));
        w.type_text("hos");
        w.type_text("tt");
        w.erase();
        assert_eq!(w.answers[0], "host");
        w.paste(".example\tcom\nsecond line");
        assert_eq!(w.answers[0], "host.examplecom", "one line, no tabs");
        assert_eq!(w.copied(), Some("host.examplecom"));
        w.clear();
        w.type_text("host");
        assert_eq!(w.problem, None);
        w.act(Action::Next);
        assert_eq!(w.page, Page::Input(1));
        assert!(w.installs_next());
        w.act(Action::Back);
        w.act(Action::Back);
        assert_eq!(w.page, Page::Welcome);
        w.act(Action::Install);
        w.act(Action::Next);
        assert_eq!(w.act(Action::Next), Outcome::Start);
        assert_eq!(w.answers_by_id()["a"], "host");
    }

    #[test]
    fn the_classic_look_asks_its_pages_last() {
        let mut w = asking();
        w.set_answer(0, "x".into());
        w.act(Action::Next);
        assert_eq!(w.page, Page::Input(0));
        w.act(Action::Next);
        assert_eq!(w.page, Page::Input(1));
        assert_eq!(w.act(Action::Next), Outcome::Start);
    }

    #[test]
    fn without_a_license_next_installs() {
        let mut w = plain(None);
        assert_eq!(w.act(Action::Next), Outcome::Start);
        assert_eq!(w.page, Page::Installing);
        assert_eq!(
            w.act(Action::Cancel),
            Outcome::Stay,
            "no cancel mid-install"
        );
    }

    #[test]
    fn license_must_be_accepted_when_asked() {
        let mut w = plain(Some(true));
        assert_eq!(w.act(Action::Next), Outcome::Stay);
        assert_eq!(w.page, Page::License);
        assert!(!w.can_go_on());
        assert_eq!(w.act(Action::Next), Outcome::Stay);
        assert_eq!(w.act(Action::Back), Outcome::Stay);
        assert_eq!(w.page, Page::Welcome);
        w.act(Action::Next);
        w.act(Action::ToggleAccept);
        assert_eq!(w.act(Action::Next), Outcome::Start);

        let mut w = plain(Some(true));
        w.act(Action::Next);
        assert_eq!(w.act(Action::Agree), Outcome::Start);
        assert!(w.accepted);
    }

    #[test]
    fn install_skips_the_license_page_once_accepted() {
        let mut w = plain(Some(true));
        assert_eq!(w.act(Action::Install), Outcome::Stay);
        assert_eq!(w.page, Page::License);
        w.act(Action::Back);
        w.act(Action::ToggleAccept);
        assert_eq!(w.act(Action::Install), Outcome::Start);
        let mut w = plain(Some(false));
        assert_eq!(w.act(Action::Install), Outcome::Start);
    }

    #[test]
    fn a_shown_license_alone_does_not_block() {
        let mut w = plain(Some(false));
        w.act(Action::Next);
        assert_eq!(w.act(Action::Next), Outcome::Start);
    }

    #[test]
    fn components_page_follows_the_license() {
        let offers = [
            component("steamvr", true, &[]),
            component("docs", false, &[]),
        ];
        let mut w = Wizard::new(Some(true), &offers, Place::default(), false);
        assert_eq!(w.chosen_ids(), ["steamvr"]);
        w.act(Action::Next);
        assert_eq!(w.act(Action::Agree), Outcome::Stay);
        assert_eq!(w.page, Page::Components);
        w.act(Action::SetComponent(0, false));
        w.act(Action::SetComponent(1, true));
        assert_eq!(w.act(Action::Back), Outcome::Stay);
        assert_eq!(w.page, Page::License);
        w.act(Action::Next);
        assert_eq!(w.act(Action::Next), Outcome::Start);
        assert_eq!(w.chosen_ids(), ["docs"]);

        let mut w = Wizard::new(None, &offers, Place::default(), false);
        w.act(Action::Next);
        assert_eq!(w.page, Page::Components);
        w.act(Action::Back);
        assert_eq!(w.page, Page::Welcome);
    }

    #[test]
    fn ticks_follow_dependencies() {
        let offers = [
            component("core-extras", false, &[]),
            component("plugins", false, &["core-extras"]),
            component("tools", true, &[]),
        ];
        let mut w = Wizard::new(None, &offers, Place::default(), false);
        w.act(Action::SetComponent(1, true));
        assert_eq!(w.chosen_ids(), ["core-extras", "plugins", "tools"]);
        w.act(Action::SetComponent(0, false));
        assert_eq!(w.chosen_ids(), ["tools"], "plugins needs core-extras");
        w.act(Action::SetComponent(9, true));
        assert_eq!(w.chosen_ids(), ["tools"]);
    }

    #[test]
    fn required_components_stay_ticked() {
        let mut driver = component("steamvr", false, &["runtime"]);
        driver.required = true;
        let offers = [component("runtime", false, &[]), driver];
        let mut w = Wizard::new(None, &offers, Place::default(), false);
        assert_eq!(
            w.chosen_ids(),
            ["runtime", "steamvr"],
            "ticked from the start"
        );
        assert!(
            w.locked(0) && w.locked(1),
            "runtime is needed by a required part"
        );
        w.act(Action::SetComponent(1, false));
        w.act(Action::SetComponent(0, false));
        assert_eq!(w.chosen_ids(), ["runtime", "steamvr"]);
    }

    #[test]
    fn modern_install_takes_the_ticks_from_welcome() {
        let mut w = Wizard::new(
            None,
            &[component("steamvr", true, &[])],
            Place::default(),
            false,
        );
        w.act(Action::SetComponent(0, false));
        assert_eq!(w.act(Action::Install), Outcome::Start);
        assert!(w.chosen_ids().is_empty());
    }

    #[test]
    fn directory_page_comes_last_and_blocks_bad_folders() {
        let offers = [component("steamvr", true, &[])];
        let start = PathBuf::from("/apps/Ozone");
        let mut w = Wizard::new(None, &offers, at(&start), true);
        w.act(Action::Next);
        assert_eq!(w.page, Page::Components);
        w.act(Action::Next);
        assert_eq!(w.page, Page::Directory);
        w.set_location(Path::new("/"), Err(LocationError::Root));
        assert_eq!(w.act(Action::Next), Outcome::Stay, "no install into /");
        assert_eq!(w.page, Page::Directory);
        w.set_location(Path::new("/opt/ozone"), Ok(()));
        assert_eq!(w.act(Action::Back), Outcome::Stay);
        assert_eq!(w.page, Page::Components);
        w.act(Action::Next);
        assert_eq!(w.act(Action::Next), Outcome::Start);
        assert_eq!(w.chosen_location(), Some(PathBuf::from("/opt/ozone")));
    }

    #[test]
    fn a_locked_folder_is_neither_shown_nor_changed() {
        let here = PathBuf::from("/apps/Ozone");
        let mut w = Wizard::new(None, &[], at(&here), false);
        w.set_location(Path::new("/elsewhere"), Ok(()));
        assert_eq!(w.location, Some(here));
        assert_eq!(w.chosen_location(), None, "upgrades stay where they are");
        assert_eq!(w.act(Action::Next), Outcome::Start);
    }

    #[test]
    fn the_scope_page_switches_folder_and_what_is_installed() {
        let mut w = Wizard::new(Some(false), &[], at(Path::new("/pf/Ozone")), true);
        let mine = Place {
            location: Some(PathBuf::from("/me/Ozone")),
            existing: Existing::Older("0.9.0".into()),
        };
        w.offer_scope(true, mine);
        w.act(Action::Next);
        w.act(Action::Next);
        assert_eq!(w.page, Page::Scope, "after the license");
        w.act(Action::Next);
        assert_eq!(w.page, Page::Directory);
        w.act(Action::Back);
        w.act(Action::SetEveryone(false));
        assert!(!w.for_everyone);
        assert_eq!(w.location, Some(PathBuf::from("/me/Ozone")));
        assert!(!w.folder_open, "an update stays in its folder");
        assert_eq!(w.act(Action::Next), Outcome::Start, "no folder page");
        w.not_started("no".into());
        assert_eq!(w.page, Page::Scope);
        assert_eq!(w.notice.as_deref(), Some("no"));
        w.act(Action::SetEveryone(true));
        assert_eq!(w.location, Some(PathBuf::from("/pf/Ozone")));
        assert!(w.folder_open && w.notice.is_none());
    }

    #[test]
    fn running_programs_are_closed_only_once_agreed() {
        let mut w = plain(Some(false));
        w.act(Action::Next);
        assert_eq!(w.act(Action::Next), Outcome::Start);
        w.must_close(vec!["Ozone".into()]);
        assert_eq!(w.page, Page::Running);
        w.act(Action::Back);
        assert_eq!(w.page, Page::License, "back where installing started");
        assert_eq!(w.act(Action::Next), Outcome::Start);
        w.must_close(vec!["Ozone".into()]);
        assert_eq!(w.act(Action::Next), Outcome::Start);
        assert!(w.close_ok);
        w.not_started("no".into());
        assert_eq!(w.page, Page::License);
    }

    #[test]
    fn a_full_drive_blocks_starting() {
        let mut big = component("docs", true, &[]);
        big.size = 600;
        let mut w = Wizard::new(None, &[big], Place::default(), false);
        w.base_size = 500;
        w.free = Some(1000);
        assert_eq!(w.space_needed(), 1100);
        assert!(w.short_of_space());
        assert_eq!(w.act(Action::Install), Outcome::Stay);
        w.act(Action::SetComponent(0, false));
        assert!(!w.short_of_space(), "fits without the docs");
        assert_eq!(w.act(Action::Install), Outcome::Start);
    }

    #[test]
    fn a_newer_install_blocks_starting() {
        let mut w = plain(None);
        w.blocked = true;
        assert_eq!(w.act(Action::Next), Outcome::Stay);
        assert_eq!(w.act(Action::Install), Outcome::Stay);
        assert_eq!(w.act(Action::Cancel), Outcome::Exit { launch: None });
    }

    #[test]
    fn finish_runs_the_app_unless_unticked() {
        let exe = PathBuf::from("app.exe");
        let mut w = plain(None);
        w.act(Action::Next);
        w.progress(0.5, Step::File("app.exe".into()));
        w.progress(0.2, Step::File("late".into()));
        assert!((w.fraction - 0.5).abs() < f32::EPSILON, "never goes back");
        w.finished(Ok(Installed {
            launch: Some(exe.clone()),
        }));
        assert_eq!(w.page, Page::Done);
        assert_eq!(w.clone_exit(), Outcome::Exit { launch: Some(exe) });
        w.act(Action::SetOpen(false));
        assert_eq!(w.act(Action::Next), Outcome::Exit { launch: None });
    }

    #[test]
    fn failure_shows_the_error_and_closes() {
        let mut w = plain(None);
        w.act(Action::Next);
        w.finished(Err("disk full".into()));
        assert_eq!(w.page, Page::Failed);
        assert_eq!(w.error.as_deref(), Some("disk full"));
        assert_eq!(w.act(Action::Next), Outcome::Exit { launch: None });
    }

    impl Wizard {
        fn clone_exit(&self) -> Outcome {
            Outcome::Exit {
                launch: self.launch.clone().filter(|_| self.open_after),
            }
        }
    }
}
