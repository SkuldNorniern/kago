//! the setup window, in two looks over one wizard: `modern` drawn on an
//! Aurea canvas like Lykil and Aira Studio, `native` a classic NSIS-style
//! wizard of system widgets. the install runs on a worker thread and
//! reports back over a channel; this module never touches the machine.

mod modern;
mod native;
mod text;
mod uninstall;
mod wizard;

pub use uninstall::{Removed, Removing, run as run_uninstall};

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use aurea::AureaResult;

use crate::format::image::Rgba;
use crate::install::LocationError;
use crate::manifest::SetupLook;
use text::{Lang, Line};
use wizard::{Action, Outcome, Wizard};

/// what the window shows about the package.
#[derive(Debug, Clone)]
pub struct Info {
    pub name: String,
    pub version: String,
    pub publisher: Option<String>,
    /// where it goes and what is there already.
    pub place: Place,
    /// installing for everyone rather than just this user.
    pub for_everyone: bool,
    /// the other way to install, when the person may pick between
    /// everyone and just them.
    pub other: Option<Place>,
    /// picked already, by the setup that asked for administrator rights:
    /// the window starts installing straight away.
    pub picked: Option<Picked>,
    pub license: Option<License>,
    /// optional parts the person can tick or untick.
    pub components: Vec<Component>,
    pub brand: Brand,
}

/// one way to install: where `{app}` goes and what is installed there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Place {
    /// where it will be installed, when known.
    pub location: Option<PathBuf>,
    /// an update stays in its folder.
    pub existing: Existing,
}

/// the same package already on this machine, by its version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Existing {
    /// not installed: a fresh install.
    #[default]
    None,
    /// this version: running setup repairs it.
    Same,
    /// an older version: setup updates it.
    Older(String),
    /// a newer version: replaced only when the package allows downgrades.
    Newer { version: String, allowed: bool },
    /// installed the other way, for everyone or just this user: setup
    /// removes that copy and installs this one.
    Elsewhere { for_everyone: bool },
}

impl Existing {
    /// installed where this setup installs, so the folder stays.
    #[must_use]
    pub const fn installed(&self) -> bool {
        !matches!(self, Self::None | Self::Elsewhere { .. })
    }

    /// a newer version the package does not let this one replace.
    #[must_use]
    pub const fn blocks(&self) -> bool {
        matches!(self, Self::Newer { allowed: false, .. })
    }

    /// the word on the button that starts the install.
    #[must_use]
    pub const fn verb(&self) -> Line {
        match self {
            Self::None | Self::Newer { .. } | Self::Elsewhere { .. } => Line::Install,
            Self::Same => Line::Repair,
            Self::Older(_) => Line::Update,
        }
    }

    /// one sentence on what setup will do, for the welcome page.
    #[must_use]
    pub fn sentence(&self, lang: Lang, name: &str, version: &str) -> Option<String> {
        let (line, from) = match self {
            Self::None => return None,
            Self::Same => (Line::StateRepair, ""),
            Self::Older(from) => (Line::StateUpdate, from.as_str()),
            Self::Newer {
                version: from,
                allowed: false,
            } => (Line::StateNewer, from.as_str()),
            Self::Newer {
                version: from,
                allowed: true,
            } => (Line::StateReplace, from.as_str()),
            Self::Elsewhere {
                for_everyone: false,
            } => (Line::StateToEveryone, ""),
            Self::Elsewhere { for_everyone: true } => (Line::StateToUser, ""),
        };
        Some(lang.fill(
            line,
            &[("name", name), ("version", version), ("from", from)],
        ))
    }
}

/// how the package dresses its setup. everything is optional.
#[derive(Debug, Clone, Default)]
pub struct Brand {
    /// small text at the bottom left, like NSIS's `BrandingText`.
    pub text: Option<String>,
    pub accent: Option<[u8; 3]>,
    pub welcome: Option<String>,
    pub finish: Option<String>,
    /// label and URL of a link on the finish page.
    pub link: Option<(String, String)>,
    /// the app's mark; the app icon unless the package gave one.
    pub logo: Option<Rgba>,
    /// top right of the classic look's inner pages.
    pub header: Option<Rgba>,
    /// along the left of the welcome and finish pages.
    pub side: Option<Rgba>,
}

/// an optional part of the package.
#[derive(Debug, Clone)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// ticked to begin with.
    pub default: bool,
    /// other components it needs.
    pub depends: Vec<String>,
    /// bytes it adds.
    pub size: u64,
    /// what it is for and was not found here, like `SteamVR`.
    pub missing: Option<String>,
    /// always installed; shown ticked and locked.
    pub required: bool,
}

impl Component {
    /// the line under or beside its name: not found, else size and
    /// description.
    fn note(&self, lang: Lang) -> (String, bool) {
        if let Some(missing) = &self.missing {
            return (lang.fill(Line::NotFound, &[("item", missing)]), true);
        }
        // a component of shortcuts or settings only has no size to show.
        let size = (self.size > 0).then(|| lang.size(self.size));
        let text = match (size, &self.description) {
            (Some(size), Some(about)) => format!("{size}  \u{b7}  {about}"),
            (Some(size), None) => size,
            (None, Some(about)) => about.clone(),
            (None, None) => String::new(),
        };
        (text, false)
    }
}

#[derive(Debug, Clone)]
pub struct License {
    pub text: String,
    /// installing needs the user to accept it.
    pub accept: bool,
}

/// how the window lets the person pick a folder. the window itself never
/// touches the disk: picking and checking come from here.
pub struct Folder {
    /// the system's folder dialog over the window with this handle,
    /// titled and starting where given.
    pub browse: FolderDialog,
    /// whether a folder can take the install, for everyone (true) or just
    /// this user.
    pub check: FolderCheck,
}

/// whether a folder can take the install, for everyone (true) or not.
pub type FolderCheck = Box<dyn Fn(&Path, bool) -> Result<(), LocationError>>;

/// starts the setup again with administrator rights, to install for
/// everyone what the person picked. `Ok` once it runs; the window closes.
pub type Elevate = Box<dyn Fn(&Picked) -> Result<(), crate::platform::ElevationError>>;

/// the system's folder dialog: owner window, title, starting folder.
pub type FolderDialog = fn(Option<isize>, &str, Option<&Path>) -> Option<PathBuf>;

/// what the person chose, for the install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub components: Vec<String>,
    /// `None` keeps the default, or the folder an update replaces.
    pub location: Option<PathBuf>,
    /// for everyone rather than just this user.
    pub for_everyone: bool,
}

/// what a finished install reports.
#[derive(Debug, Clone)]
pub struct Installed {
    /// main executable to offer a launch for.
    pub launch: Option<PathBuf>,
}

/// one thing the install is doing, for the details list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Unpack(String),
    Folder(String),
    File(String),
    Path(String),
    SteamVr(String),
    Shortcut(String),
    Register(String),
    Hook(String),
    Service(String),
}

impl Step {
    fn text(&self, lang: Lang) -> String {
        let (line, item) = match self {
            Self::Unpack(s) => (Line::Unpack, s),
            Self::Folder(s) => (Line::Folder, s),
            Self::File(s) => (Line::File, s),
            Self::Path(s) => (Line::Path, s),
            Self::SteamVr(s) => (Line::SteamVr, s),
            Self::Shortcut(s) => (Line::Shortcut, s),
            Self::Register(s) => (Line::Register, s),
            Self::Hook(s) => (Line::Hook, s),
            Self::Service(s) => (Line::Service, s),
        };
        lang.fill(line, &[("item", item)])
    }
}

/// handed to the install so it can say how far it is.
#[derive(Clone)]
pub struct Reporter(Sender<Event>);

impl Reporter {
    /// `fraction` is 0 to 1 over the whole install.
    pub fn report(&self, fraction: f32, step: Step) {
        let _ = self.0.send(Event::Progress(fraction, step));
    }
}

enum Event {
    Progress(f32, Step),
    Finished(Result<Installed, String>),
}

/// shows the setup until the user closes it. `install` runs once, on a
/// worker thread, when the wizard gets to installing, with what the person
/// picked. `folder` lets them choose where; without it the folder is fixed.
/// `elevate`, when given, takes over an install for everyone. returns
/// whether an install finished or was handed over.
///
/// # Errors
///
/// when the window cannot be created.
pub fn run<F>(
    look: SetupLook,
    info: &Info,
    folder: Option<Folder>,
    elevate: Option<Elevate>,
    install: F,
) -> AureaResult<bool>
where
    F: FnOnce(&Reporter, Picked) -> Result<Installed, String> + Send + 'static,
{
    let lang = Lang::detect();
    let mut session = Session::new(info, folder, install);
    session.elevate = elevate;
    session.lang = lang;
    if let Some(picked) = &info.picked {
        session.resume(picked);
    }
    match look {
        SetupLook::Modern => modern::run(info, lang, session),
        SetupLook::Native => native::run(info, lang, session),
    }
}

type Work = Box<dyn FnOnce(&Reporter, Picked) -> Result<Installed, String> + Send>;

/// the wizard and the install it starts. both looks feed it actions and
/// pump it once per frame.
struct Session {
    wizard: Wizard,
    folder: Option<Folder>,
    /// the folder name the app gets inside a picked folder.
    app_folder: String,
    install: Option<Work>,
    /// hands an install for everyone to an elevated setup.
    elevate: Option<Elevate>,
    /// the install went to the elevated setup.
    handed_off: bool,
    lang: Lang,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Session {
    fn new<F>(info: &Info, folder: Option<Folder>, install: F) -> Self
    where
        F: FnOnce(&Reporter, Picked) -> Result<Installed, String> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let mut wizard = Wizard::new(
            info.license.as_ref().map(|l| l.accept),
            &info.components,
            info.place.clone(),
            folder.is_some(),
        );
        wizard.for_everyone = info.for_everyone;
        if let Some(other) = &info.other {
            wizard.offer_scope(info.for_everyone, other.clone());
        }
        Self {
            wizard,
            folder,
            app_folder: crate::install::folder_name(&info.name),
            install: Some(Box::new(install)),
            elevate: None,
            handed_off: false,
            lang: Lang::En,
            tx,
            rx,
        }
    }

    /// takes what the setup that asked for administrator rights was given
    /// and starts installing.
    fn resume(&mut self, picked: &Picked) {
        self.wizard.accepted = true;
        self.wizard.pick(&picked.components);
        if let Some(dir) = &picked.location {
            self.set_location(dir);
        }
        // blocked or an unusable folder stays on the welcome page.
        let outcome = self.wizard.install_now();
        self.follow(outcome);
    }

    /// applies `action`. true when the window should close.
    fn act(&mut self, action: Action) -> bool {
        let outcome = self.wizard.act(action);
        self.follow(outcome)
    }

    /// does what the wizard's answer asks. true when the window should
    /// close.
    fn follow(&mut self, outcome: Outcome) -> bool {
        match outcome {
            Outcome::Stay => false,
            Outcome::Start => {
                let picked = Picked {
                    components: self.wizard.chosen_ids(),
                    location: self.wizard.chosen_location(),
                    for_everyone: self.wizard.for_everyone,
                };
                if picked.for_everyone
                    && let Some(elevate) = &self.elevate
                {
                    return match elevate(&picked) {
                        Ok(()) => {
                            self.handed_off = true;
                            true
                        }
                        Err(e) => {
                            let why = match e {
                                crate::platform::ElevationError::Declined => {
                                    self.lang.line(Line::AdminDeclined).to_owned()
                                }
                                crate::platform::ElevationError::Failed(e) => e,
                            };
                            self.wizard.not_started(why);
                            false
                        }
                    };
                }
                if let Some(work) = self.install.take() {
                    let reporter = Reporter(self.tx.clone());
                    thread::spawn(move || {
                        let result = work(&reporter, picked);
                        let _ = reporter.0.send(Event::Finished(result));
                    });
                }
                false
            }
            Outcome::Exit { launch } => {
                if let Some(exe) = launch {
                    // through Explorer on Windows: an app started by a setup
                    // running as administrator would run as administrator too.
                    let mut cmd = if cfg!(windows) {
                        let mut explorer = std::process::Command::new("explorer");
                        explorer.arg(&exe);
                        explorer
                    } else {
                        std::process::Command::new(&exe)
                    };
                    if let Some(dir) = exe.parent() {
                        cmd.current_dir(dir);
                    }
                    let _ = cmd.spawn();
                }
                true
            }
        }
    }

    /// takes a typed or picked folder and checks it.
    fn set_location(&mut self, dir: &Path) {
        let everyone = self.wizard.for_everyone;
        let check = self
            .folder
            .as_ref()
            .map_or(Ok(()), |f| (f.check)(dir, everyone));
        self.wizard.set_location(dir, check);
    }

    /// the folder dialog and where it should start, for a caller that has
    /// to open it outside its own borrow of the session.
    fn browser(&self) -> Option<(FolderDialog, Option<PathBuf>)> {
        let folder = self.folder.as_ref()?;
        self.wizard
            .folder_open
            .then(|| (folder.browse, self.wizard.location.clone()))
    }

    /// a folder from the dialog. like NSIS, the app gets its own folder in
    /// it, unless the person picked one named for the app already.
    fn picked(&mut self, dir: &Path) {
        let own = dir
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case(self.app_folder.as_str()));
        let dir = if own {
            dir.to_path_buf()
        } else {
            dir.join(&self.app_folder)
        };
        self.set_location(&dir);
    }

    /// opens the folder dialog over `owner`. true when a folder was picked.
    fn browse(&mut self, owner: Option<isize>, title: &str) -> bool {
        let Some(folder) = &self.folder else {
            return false;
        };
        let start = self.wizard.location.clone();
        match (folder.browse)(owner, title, start.as_deref()) {
            Some(dir) => {
                self.picked(&dir);
                true
            }
            None => false,
        }
    }

    /// takes what the install reported. true when anything changed.
    fn pump(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.rx.try_recv() {
            changed = true;
            match event {
                Event::Progress(fraction, step) => self.wizard.progress(fraction, step),
                Event::Finished(result) => self.wizard.finished(result),
            }
        }
        changed
    }

    /// opens the brand link in the browser.
    fn open_link(url: &str) {
        let opener = if cfg!(windows) {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        // only web links; nothing else gets handed to the shell.
        if url.starts_with("https://") || url.starts_with("http://") {
            let _ = std::process::Command::new(opener).arg(url).spawn();
        }
    }

    /// whether the install has finished, or went to the elevated setup,
    /// for the exit code.
    fn installed(&self) -> bool {
        self.handed_off || self.wizard.page == wizard::Page::Done
    }

    fn closable(&self) -> bool {
        self.wizard.page != wizard::Page::Installing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> Info {
        Info {
            name: "Ozone".into(),
            version: "1.0.0".into(),
            publisher: None,
            place: Place {
                location: Some(PathBuf::from("/apps/Ozone")),
                existing: Existing::None,
            },
            for_everyone: false,
            other: None,
            picked: None,
            license: None,
            components: Vec::new(),
            brand: Brand::default(),
        }
    }

    fn session(dialog: FolderDialog) -> Session {
        let folder = Folder {
            browse: dialog,
            check: Box::new(|_, _| Ok(())),
        };
        Session::new(&info(), Some(folder), |_, _| Ok(Installed { launch: None }))
    }

    #[test]
    fn a_picked_folder_gets_the_app_folder_inside() {
        let mut s = session(|_, _, _| Some(PathBuf::from("/data/tools")));
        assert!(s.browse(None, "pick"));
        assert_eq!(s.wizard.location, Some(PathBuf::from("/data/tools/Ozone")));

        let mut s = session(|_, _, _| Some(PathBuf::from("/data/ozone")));
        s.browse(None, "pick");
        assert_eq!(
            s.wizard.location,
            Some(PathBuf::from("/data/ozone")),
            "already its own"
        );

        let mut s = session(|_, _, _| None);
        assert!(!s.browse(None, "pick"), "cancelled");
        assert_eq!(s.wizard.location, Some(PathBuf::from("/apps/Ozone")));
    }
}
