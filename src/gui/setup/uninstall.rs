//! the uninstaller's window, like an NSIS uninstaller: asks first, shows it
//! working, then says what is gone and what stayed. it takes the look of
//! the setup that installed the package. the removal itself runs on a
//! worker thread and reports back over a channel; this module never
//! touches the machine.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use aurea::elements::{
    Button, Checkbox, Container, Divider, Label, Orientation, ProgressBar, Stack, TextView,
};
use aurea::{AureaResult, Window, WindowEvent};

use super::text::{Lang, Line};
use crate::format::image::Rgba;
use crate::manifest::SetupLook;

const WIDTH: i32 = 480;
const HEIGHT: i32 = 300;

/// what the window says about the installed package.
#[derive(Debug, Clone)]
pub struct Removing {
    pub name: String,
    pub version: String,
    pub location: Option<PathBuf>,
    /// the look of the setup that installed it.
    pub look: SetupLook,
    pub accent: Option<[u8; 3]>,
    /// small text at the bottom left.
    pub text: Option<String>,
    /// the app's icon, for the modern look.
    pub logo: Option<Rgba>,
    /// programs using its files, which removing closes.
    pub running: Vec<String>,
}

/// what an uninstall left behind.
#[derive(Debug, Clone, Default)]
pub struct Removed {
    /// user data and changed files, kept on purpose.
    pub kept: Vec<String>,
    /// files that could not be removed, like one still in use.
    pub failed: Vec<String>,
    /// in use, so removed when the computer next starts.
    pub at_restart: Vec<String>,
}

impl Removing {
    /// which programs removing closes, when any run.
    pub(super) fn running_note(&self, lang: Lang) -> Option<String> {
        (!self.running.is_empty()).then(|| {
            lang.fill(
                Line::UninstallRunning,
                &[("item", &self.running.join(", "))],
            )
        })
    }
}

impl Removed {
    /// removed, or removed but for what is listed.
    pub(super) const fn headline(&self) -> Line {
        if !self.failed.is_empty() {
            Line::RemovedPartly
        } else if self.at_restart.is_empty() {
            Line::Removed
        } else {
            Line::RemovedRestart
        }
    }
}

pub(super) enum Page {
    Ask,
    Working,
    Done(Removed),
    Failed(String),
}

/// removes the package; true also removes the user's settings and data.
type Work = Box<dyn FnOnce(bool) -> Result<Removed, String> + Send>;

/// the removal and how far it is, the same for both looks.
pub(super) struct Removal {
    pub page: Page,
    /// the person ticked removing their settings and data too.
    pub purge: bool,
    work: Option<Work>,
    tx: Sender<Result<Removed, String>>,
    rx: Receiver<Result<Removed, String>>,
}

impl Removal {
    fn new(work: Work) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            page: Page::Ask,
            purge: false,
            work: Some(work),
            tx,
            rx,
        }
    }

    /// the person said yes: removing starts.
    pub fn confirm(&mut self) {
        if let (Page::Ask, Some(work)) = (&self.page, self.work.take()) {
            let tx = self.tx.clone();
            let purge = self.purge;
            thread::spawn(move || {
                let _ = tx.send(work(purge));
            });
            self.page = Page::Working;
        }
    }

    /// takes the result once there is one. true when the page changed.
    pub fn pump(&mut self) -> bool {
        match self.rx.try_recv() {
            Ok(Ok(done)) => self.page = Page::Done(done),
            Ok(Err(e)) => self.page = Page::Failed(e),
            Err(_) => return false,
        }
        true
    }

    pub const fn closable(&self) -> bool {
        !matches!(self.page, Page::Working)
    }

    pub const fn removed(&self) -> bool {
        matches!(self.page, Page::Done(_))
    }

    /// what stayed and what could not go, one per line, under headings.
    pub fn notes(lang: Lang, done: &Removed) -> Vec<String> {
        let mut notes = Vec::new();
        if !done.failed.is_empty() {
            notes.push(lang.line(Line::NotRemoved).to_owned());
            notes.extend(done.failed.iter().cloned());
        }
        if !done.at_restart.is_empty() {
            notes.push(lang.line(Line::AtRestart).to_owned());
            notes.extend(done.at_restart.iter().cloned());
        }
        if !done.kept.is_empty() {
            notes.push(lang.line(Line::Kept).to_owned());
            notes.extend(done.kept.iter().cloned());
        }
        notes
    }
}

/// shows the uninstaller until it is closed. `remove` runs once, on a
/// worker thread, after the person confirms. returns whether the package
/// was removed.
///
/// # Errors
///
/// when the window cannot be created.
pub fn run<F>(info: &Removing, remove: F) -> AureaResult<bool>
where
    F: FnOnce(bool) -> Result<Removed, String> + Send + 'static,
{
    let lang = Lang::detect();
    let removal = Removal::new(Box::new(remove));
    match info.look {
        SetupLook::Modern => super::modern::uninstall(info, lang, removal),
        SetupLook::Native => native(info, lang, removal),
    }
}

enum Message {
    Remove,
    Close,
}

fn native(info: &Removing, lang: Lang, mut removal: Removal) -> AureaResult<bool> {
    let mut window = Window::new(
        &lang.fill(Line::UninstallTitle, &[("name", &info.name)]),
        WIDTH,
        HEIGHT,
    )?;
    let (tx, rx) = mpsc::channel();
    let (content, mut purge) = render(info, lang, &removal.page, &tx)?;
    window.set_content(content)?;
    window.show();
    loop {
        for event in window.wait_events(Some(super::PUMP)) {
            if matches!(event, WindowEvent::CloseRequested) && removal.closable() {
                return Ok(removal.removed());
            }
        }
        let changed = match rx.try_recv() {
            Ok(Message::Remove) => {
                removal.purge = purge.take().is_some_and(|b| b.get_checked());
                removal.confirm();
                true
            }
            Ok(Message::Close) if removal.closable() => return Ok(removal.removed()),
            _ => removal.pump(),
        };
        if changed {
            let (content, next) = render(info, lang, &removal.page, &tx)?;
            window.set_content(content)?;
            purge = next;
        }
    }
}

fn button(title: &str, tx: &Sender<Message>, message: fn() -> Message) -> AureaResult<Button> {
    let tx = tx.clone();
    Button::with_callback(title, move || {
        let _ = tx.send(message());
    })
}

/// the page, and on the asking page the box for removing the user's
/// settings and data too.
fn render(
    info: &Removing,
    lang: Lang,
    page: &Page,
    tx: &Sender<Message>,
) -> AureaResult<(Stack, Option<std::rc::Rc<Checkbox>>)> {
    let mut purge = None;
    let fill = |line| {
        lang.fill(
            line,
            &[("name", info.name.as_str()), ("version", &info.version)],
        )
    };
    let mut window = Stack::new(Orientation::Vertical)?;
    let mut body = Stack::new(Orientation::Vertical)?;
    let mut buttons = Stack::new(Orientation::Horizontal)?;
    buttons.add(Label::new(info.text.as_deref().unwrap_or("Kago"))?)?;
    buttons.add_spacer(1.0)?;
    match page {
        Page::Ask => {
            body.add(Label::new(&fill(Line::UninstallTitle))?)?;
            body.add(Label::new(&fill(Line::UninstallAsk))?)?;
            if let Some(location) = &info.location {
                body.add(Label::new(&format!("    {}", location.display()))?)?;
            }
            if let Some(note) = info.running_note(lang) {
                body.add(Label::new(&note)?)?;
            }
            let tick =
                std::rc::Rc::new(Checkbox::with_checked(lang.line(Line::RemoveData), false)?);
            body.add(Shared(std::rc::Rc::clone(&tick)))?;
            purge = Some(tick);
            body.add_spacer(1.0)?;
            buttons.add(button(lang.line(Line::Uninstall), tx, || Message::Remove)?)?;
            buttons.add(button(lang.line(Line::Cancel), tx, || Message::Close)?)?;
        }
        Page::Working => {
            body.add(Label::new(&fill(Line::Removing))?)?;
            let bar = ProgressBar::with_indeterminate(true)?;
            bar.start_animation()?;
            body.add(bar)?;
            body.add_spacer(1.0)?;
        }
        Page::Done(done) => {
            body.add(Label::new(&fill(done.headline()))?)?;
            let notes = Removal::notes(lang, done);
            if notes.is_empty() {
                body.add_spacer(1.0)?;
            } else {
                body.add_weighted(TextView::with_content(false, &notes.join("\n"))?, 1.0)?;
            }
            buttons.add(button(lang.line(Line::Close), tx, || Message::Close)?)?;
        }
        Page::Failed(error) => {
            body.add(Label::new(lang.line(Line::UninstallFailed))?)?;
            body.add_weighted(TextView::with_content(false, error)?, 1.0)?;
            buttons.add(button(lang.line(Line::Close), tx, || Message::Close)?)?;
        }
    }
    window.add_weighted(body, 1.0)?;
    window.add(Divider::horizontal(WIDTH.unsigned_abs())?)?;
    window.add(buttons)?;
    Ok((window, purge))
}

/// a box still readable after the page took it.
struct Shared(std::rc::Rc<Checkbox>);

#[allow(
    unsafe_code,
    reason = "Element's invalidate is an unsafe trait method; this only forwards it"
)]
impl aurea::elements::Element for Shared {
    fn handle(&self) -> *mut std::os::raw::c_void {
        self.0.handle()
    }

    fn released_to_parent(&self) {
        self.0.released_to_parent();
    }

    unsafe fn invalidate_platform(&self, rect: Option<aurea::render::Rect>) {
        // SAFETY: same element, same thread; the caller's guarantees hold.
        unsafe { self.0.invalidate_platform(rect) }
    }
}
