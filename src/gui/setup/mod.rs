//! the setup window, in two looks over one wizard: `modern` drawn on an
//! Aurea canvas like Lykil and Aira Studio, `native` a classic NSIS-style
//! wizard of system widgets. the install runs on a worker thread and
//! reports back over a channel; this module never touches the machine.

mod modern;
mod native;
mod text;
mod wizard;

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use aurea::AureaResult;

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
    pub license: Option<License>,
}

#[derive(Debug, Clone)]
pub struct License {
    pub text: String,
    /// installing needs the user to accept it.
    pub accept: bool,
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
    Shortcut(String),
    Register(String),
    Service(String),
}

impl Step {
    fn text(&self, lang: Lang) -> String {
        let (line, item) = match self {
            Self::Unpack(s) => (Line::Unpack, s),
            Self::Folder(s) => (Line::Folder, s),
            Self::File(s) => (Line::File, s),
            Self::Path(s) => (Line::Path, s),
            Self::Shortcut(s) => (Line::Shortcut, s),
            Self::Register(s) => (Line::Register, s),
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
/// worker thread, when the wizard gets to installing. returns whether an
/// install finished.
///
/// # Errors
///
/// when the window cannot be created.
pub fn run<F>(look: SetupLook, info: &Info, install: F) -> AureaResult<bool>
where
    F: FnOnce(&Reporter) -> Result<Installed, String> + Send + 'static,
{
    let lang = Lang::detect();
    let session = Session::new(info, install);
    match look {
        SetupLook::Modern => modern::run(info, lang, session),
        SetupLook::Native => native::run(info, lang, session),
    }
}

type Work = Box<dyn FnOnce(&Reporter) -> Result<Installed, String> + Send>;

/// the wizard and the install it starts. both looks feed it actions and
/// pump it once per frame.
struct Session {
    wizard: Wizard,
    install: Option<Work>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

impl Session {
    fn new<F>(info: &Info, install: F) -> Self
    where
        F: FnOnce(&Reporter) -> Result<Installed, String> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        Self {
            wizard: Wizard::new(info.license.as_ref().map(|l| l.accept)),
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
                    thread::spawn(move || {
                        let result = work(&reporter);
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
