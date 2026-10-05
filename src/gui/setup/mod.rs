//! the setup window, in two looks over one wizard: `modern` drawn on an
//! Aurea canvas like Lykil and Aira Studio, `native` a classic NSIS-style
//! wizard of system widgets. the install runs on a worker thread and
//! reports back over a channel; this module never touches the machine.

mod modern;
mod native;
mod text;
mod wizard;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use aurea::AureaResult;

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
    /// where it will be installed, when known.
    pub location: Option<PathBuf>,
    /// `location` is where the installed version already is; an update
    /// stays there.
    pub upgrade: bool,
    pub license: Option<License>,
    /// optional parts the person can tick or untick.
    pub components: Vec<Component>,
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
    /// whether a folder can take the install.
    pub check: FolderCheck,
}

/// whether a folder can take the install.
pub type FolderCheck = Box<dyn Fn(&Path) -> Result<(), LocationError>>;

/// the system's folder dialog: owner window, title, starting folder.
pub type FolderDialog = fn(Option<isize>, &str, Option<&Path>) -> Option<PathBuf>;

/// what the person chose, for the install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub components: Vec<String>,
    /// `None` keeps the default, or the folder an update replaces.
    pub location: Option<PathBuf>,
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
/// returns whether an install finished.
///
/// # Errors
///
/// when the window cannot be created.
pub fn run<F>(look: SetupLook, info: &Info, folder: Option<Folder>, install: F) -> AureaResult<bool>
where
    F: FnOnce(&Reporter, Picked) -> Result<Installed, String> + Send + 'static,
{
    let lang = Lang::detect();
    let session = Session::new(info, folder, install);
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
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Session {
    fn new<F>(info: &Info, folder: Option<Folder>, install: F) -> Self
    where
        F: FnOnce(&Reporter, Picked) -> Result<Installed, String> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        Self {
            wizard: Wizard::new(
                info.license.as_ref().map(|l| l.accept),
                &info.components,
                info.location.clone(),
                folder.is_some() && !info.upgrade,
            ),
            folder,
            app_folder: crate::install::folder_name(&info.name),
            install: Some(Box::new(install)),
            tx,
            rx,
        }
    }

    /// applies `action`. true when the window should close.
    fn act(&mut self, action: Action) -> bool {
        match self.wizard.act(action) {
            Outcome::Stay => false,
            Outcome::Start => {
                if let Some(work) = self.install.take() {
                    let reporter = Reporter(self.tx.clone());
                    let picked = Picked {
                        components: self.wizard.chosen_ids(),
                        location: self.wizard.chosen_location(),
                    };
                    thread::spawn(move || {
                        let result = work(&reporter, picked);
                        let _ = reporter.0.send(Event::Finished(result));
                    });
                }
                false
            }
            Outcome::Exit { launch } => {
                if let Some(exe) = launch {
                    let mut cmd = std::process::Command::new(&exe);
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
        let check = self.folder.as_ref().map_or(Ok(()), |f| (f.check)(dir));
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

    /// whether the install has finished, for the exit code.
    fn installed(&self) -> bool {
        self.wizard.page == wizard::Page::Done
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
            location: Some(PathBuf::from("/apps/Ozone")),
            upgrade: false,
            license: None,
            components: Vec::new(),
        }
    }

    fn session(dialog: FolderDialog) -> Session {
        let folder = Folder {
            browse: dialog,
            check: Box::new(|_| Ok(())),
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
