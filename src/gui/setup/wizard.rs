//! which page the setup is on and what each button does there. both looks
//! drive the same wizard, so the flow is the same whichever one draws it.

use std::path::{Path, PathBuf};

use super::{Component, Existing, Installed, Place, Step};
use crate::install::LocationError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Welcome,
    License,
    /// for everyone or just this user.
    Scope,
    Components,
    Directory,
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
            folder_error: None,
            fraction: 0.0,
            log: Vec::new(),
            launch: None,
            open_after: true,
            error: None,
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
        pages
    }

    /// whether Next on this page starts installing.
    #[must_use]
    pub fn installs_next(&self) -> bool {
        self.pages().last() == Some(&self.page)
    }

    /// the page after this one, or installing after the last.
    fn next(&mut self) -> Outcome {
        let pages = self.pages();
        let at = pages.iter().position(|p| *p == self.page);
        match at.and_then(|i| pages.get(i + 1)) {
            Some(page) => self.show(*page),
            None => self.start(),
        }
    }

    fn back(&mut self) -> Outcome {
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
            Page::Welcome | Page::License | Page::Scope | Page::Components | Page::Directory
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
                    self.start()
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
        Outcome::Stay
    }

    /// installing begins unless the folder is unusable.
    fn start(&mut self) -> Outcome {
        if self.folder_error.is_some() || self.blocked {
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
