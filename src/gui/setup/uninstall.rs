//! the uninstaller's window, like an NSIS uninstaller: asks first, shows it
//! working, then says what is gone and what stayed. the removal itself runs
//! on a worker thread and reports back over a channel; this module never
//! touches the machine.

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use aurea::elements::{
    Button, Container, Divider, Label, Orientation, ProgressBar, Stack, TextView,
};
use aurea::{AureaResult, Window, WindowEvent};

use super::text::{Lang, Line};

const WIDTH: i32 = 480;
const HEIGHT: i32 = 300;

/// what the window says about the installed package.
#[derive(Debug, Clone)]
pub struct Removing {
    pub name: String,
    pub version: String,
    pub location: Option<PathBuf>,
}

/// what an uninstall left behind.
#[derive(Debug, Clone, Default)]
pub struct Removed {
    /// user data and changed files, kept on purpose.
    pub kept: Vec<String>,
    /// files that could not be removed, like one still in use.
    pub failed: Vec<String>,
}

enum Message {
    Remove,
    Finished(Result<Removed, String>),
    Close,
}

enum Page {
    Ask,
    Working,
    Done(Removed),
    Failed(String),
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
    F: FnOnce() -> Result<Removed, String> + Send + 'static,
{
    let lang = Lang::detect();
    let mut window = Window::new(
        &lang.fill(Line::UninstallTitle, &[("name", &info.name)]),
        WIDTH,
        HEIGHT,
    )?;
    let (tx, rx) = mpsc::channel();
    let mut remove = Some(remove);
    let mut page = Page::Ask;
    window.set_content(render(info, lang, &page, &tx)?)?;
    window.show();
    loop {
        for event in window.poll_events() {
            if matches!(event, WindowEvent::CloseRequested) && !matches!(page, Page::Working) {
                return Ok(matches!(page, Page::Done(_)));
            }
        }
        window.process_frames()?;
        let next = match (rx.try_recv(), &page) {
            (Ok(Message::Remove), Page::Ask) => remove.take().map(|work| {
                let tx = tx.clone();
                thread::spawn(move || {
                    let _ = tx.send(Message::Finished(work()));
                });
                Page::Working
            }),
            (Ok(Message::Finished(Ok(done))), Page::Working) => Some(Page::Done(done)),
            (Ok(Message::Finished(Err(e))), Page::Working) => Some(Page::Failed(e)),
            (Ok(Message::Close), Page::Ask | Page::Done(_) | Page::Failed(_)) => {
                return Ok(matches!(page, Page::Done(_)));
            }
            (Ok(_), _) => None,
            (Err(_), _) => {
                thread::sleep(Duration::from_millis(15));
                None
            }
        };
        if let Some(next) = next {
            page = next;
            window.set_content(render(info, lang, &page, &tx)?)?;
        }
    }
}

fn button(title: &str, tx: &Sender<Message>, message: fn() -> Message) -> AureaResult<Button> {
    let tx = tx.clone();
    Button::with_callback(title, move || {
        let _ = tx.send(message());
    })
}

fn render(info: &Removing, lang: Lang, page: &Page, tx: &Sender<Message>) -> AureaResult<Stack> {
    let fill = |line| {
        lang.fill(
            line,
            &[("name", info.name.as_str()), ("version", &info.version)],
        )
    };
    let mut window = Stack::new(Orientation::Vertical)?;
    let mut body = Stack::new(Orientation::Vertical)?;
    let mut buttons = Stack::new(Orientation::Horizontal)?;
    buttons.add(Label::new("Kago")?)?;
    buttons.add_spacer(1.0)?;
    match page {
        Page::Ask => {
            body.add(Label::new(&fill(Line::UninstallTitle))?)?;
            body.add(Label::new(&fill(Line::UninstallAsk))?)?;
            if let Some(location) = &info.location {
                body.add(Label::new(&format!("    {}", location.display()))?)?;
            }
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
            body.add(Label::new(&fill(Line::Removed))?)?;
            let mut notes = Vec::new();
            if !done.failed.is_empty() {
                notes.push(lang.line(Line::NotRemoved).to_owned());
                notes.extend(done.failed.iter().cloned());
            }
            if !done.kept.is_empty() {
                notes.push(lang.line(Line::Kept).to_owned());
                notes.extend(done.kept.iter().cloned());
            }
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
    Ok(window)
}
