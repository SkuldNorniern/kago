//! which page the setup is on and what each button does there. both looks
//! drive the same wizard, so the flow is the same whichever one draws it.

use std::path::PathBuf;

use super::{Component, Installed, Step};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Welcome,
    License,
    Components,
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
    /// tick or untick the optional component at this index.
    SetComponent(usize, bool),
    /// run the app after finishing.
    SetOpen(bool),
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
    /// 0 to 1.
    pub fraction: f32,
    pub log: Vec<Step>,
    pub launch: Option<PathBuf>,
    pub open_after: bool,
    pub error: Option<String>,
}

impl Wizard {
    #[must_use]
    pub fn new(license: Option<bool>, components: &[Component]) -> Self {
        Self {
            page: Page::Welcome,
            has_license: license.is_some(),
            must_accept: license.unwrap_or(false),
            accepted: false,
            chosen: components.iter().map(|c| c.default).collect(),
            offers: components
                .iter()
                .map(|c| Offer {
                    id: c.id.clone(),
                    depends: c.depends.clone(),
                })
                .collect(),
            fraction: 0.0,
            log: Vec::new(),
            launch: None,
            open_after: true,
            error: None,
        }
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

    pub fn act(&mut self, action: Action) -> Outcome {
        match (self.page, action) {
            (Page::Installing, _) => Outcome::Stay,
            (Page::Welcome, Action::Next) | (Page::Components, Action::Back)
                if self.has_license =>
            {
                self.show(Page::License)
            }
            (Page::Welcome, Action::Next) => self.after_license(),
            (Page::License, Action::Agree) => {
                self.accepted = true;
                self.after_license()
            }
            (Page::License, Action::Next) if self.can_go_on() => self.after_license(),
            (Page::Components, Action::Next) => self.start(),
            (Page::Welcome | Page::License, Action::Install) => {
                if self.accepted || !self.must_accept {
                    self.start()
                } else {
                    self.show(Page::License)
                }
            }
            (Page::License | Page::Components, Action::Back) => self.show(Page::Welcome),
            (Page::Welcome | Page::License, Action::ToggleAccept) => {
                self.accepted = !self.accepted;
                Outcome::Stay
            }
            (Page::Welcome | Page::License | Page::Components, Action::SetComponent(i, on)) => {
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
    fn set_component(&mut self, index: usize, on: bool) {
        let Some(offer) = self.offers.get(index) else {
            return;
        };
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

    fn after_license(&mut self) -> Outcome {
        if self.offers.is_empty() {
            self.start()
        } else {
            self.show(Page::Components)
        }
    }

    fn start(&mut self) -> Outcome {
        self.page = Page::Installing;
        Outcome::Start
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
        Wizard::new(license, &[])
    }

    fn component(id: &str, default: bool, depends: &[&str]) -> Component {
        Component {
            id: id.into(),
            name: id.into(),
            description: None,
            default,
            depends: depends.iter().map(|d| (*d).to_owned()).collect(),
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
        let mut w = Wizard::new(Some(true), &offers);
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

        let mut w = Wizard::new(None, &offers);
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
        let mut w = Wizard::new(None, &offers);
        w.act(Action::SetComponent(1, true));
        assert_eq!(w.chosen_ids(), ["core-extras", "plugins", "tools"]);
        w.act(Action::SetComponent(0, false));
        assert_eq!(w.chosen_ids(), ["tools"], "plugins needs core-extras");
        w.act(Action::SetComponent(9, true));
        assert_eq!(w.chosen_ids(), ["tools"]);
    }

    #[test]
    fn modern_install_takes_the_ticks_from_welcome() {
        let mut w = Wizard::new(None, &[component("steamvr", true, &[])]);
        w.act(Action::SetComponent(0, false));
        assert_eq!(w.act(Action::Install), Outcome::Start);
        assert!(w.chosen_ids().is_empty());
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
