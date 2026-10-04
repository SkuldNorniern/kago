//! setup wizard on Aurea native widgets: welcome, installing, done or
//! failed. the install itself runs on a worker thread and reports back
//! over a channel; this module never touches the machine.

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use aurea::elements::{Button, Container, Label, Orientation, ProgressBar, Stack, TextView};
use aurea::{AureaResult, Window, WindowEvent};

/// what the welcome screen shows.
#[derive(Debug, Clone)]
pub struct Info {
    pub name: String,
    pub version: String,
    pub publisher: Option<String>,
    /// where it will be installed, when known.
    pub location: Option<PathBuf>,
}

/// what a finished install reports.
#[derive(Debug, Clone)]
pub struct Installed {
    /// main executable to offer a launch for.
    pub launch: Option<PathBuf>,
}

enum Message {
    Install,
    Finished(Result<Installed, String>),
    Launch,
    Close,
}

enum Screen {
    Welcome,
    Installing,
    Done(Installed),
    Failed(String),
}

/// shows the wizard until the user closes it. `install` runs once, on a
/// worker thread, when the user presses Install. returns whether an install
/// finished.
///
/// # Errors
///
/// when the window cannot be created.
pub fn run<F>(info: &Info, install: F) -> AureaResult<bool>
where
    F: FnOnce() -> Result<Installed, String> + Send + 'static,
{
    let mut window = Window::new(&format!("{} Setup", info.name), 480, 260)?;
    let (tx, rx) = mpsc::channel();
    let mut install = Some(install);
    let mut screen = Screen::Welcome;
    window.set_content(render(info, &screen, &tx)?)?;
    window.show();
    loop {
        for event in window.poll_events() {
            if matches!(event, WindowEvent::CloseRequested) && !matches!(screen, Screen::Installing)
            {
                return Ok(matches!(screen, Screen::Done(_)));
            }
        }
        window.process_frames()?;
        let next = match rx.try_recv() {
            Ok(message) => step(&screen, message, &mut install, &tx),
            Err(_) => None,
        };
        match next {
            Some(Step::Show(next)) => {
                screen = next;
                window.set_content(render(info, &screen, &tx)?)?;
            }
            Some(Step::Exit) => return Ok(matches!(screen, Screen::Done(_))),
            None => thread::sleep(Duration::from_millis(15)),
        }
    }
}

enum Step {
    Show(Screen),
    Exit,
}

fn step<F>(
    screen: &Screen,
    message: Message,
    install: &mut Option<F>,
    tx: &Sender<Message>,
) -> Option<Step>
where
    F: FnOnce() -> Result<Installed, String> + Send + 'static,
{
    match (screen, message) {
        (Screen::Welcome, Message::Install) => {
            let work = install.take()?;
            let tx = tx.clone();
            thread::spawn(move || {
                let _ = tx.send(Message::Finished(work()));
            });
            Some(Step::Show(Screen::Installing))
        }
        (Screen::Installing, Message::Finished(Ok(done))) => Some(Step::Show(Screen::Done(done))),
        (Screen::Installing, Message::Finished(Err(e))) => Some(Step::Show(Screen::Failed(e))),
        (Screen::Done(done), Message::Launch) => {
            if let Some(exe) = &done.launch {
                let mut cmd = std::process::Command::new(exe);
                if let Some(dir) = exe.parent() {
                    cmd.current_dir(dir);
                }
                let _ = cmd.spawn();
            }
            Some(Step::Exit)
        }
        (Screen::Installing, _) => None,
        (_, Message::Close) => Some(Step::Exit),
        _ => None,
    }
}

fn button(title: &str, tx: &Sender<Message>, message: fn() -> Message) -> AureaResult<Button> {
    let tx = tx.clone();
    Button::with_callback(title, move || {
        let _ = tx.send(message());
    })
}

fn render(info: &Info, screen: &Screen, tx: &Sender<Message>) -> AureaResult<Stack> {
    let mut page = Stack::new(Orientation::Vertical)?;
    let mut buttons = Stack::new(Orientation::Horizontal)?;
    buttons.add_spacer(1.0)?;
    match screen {
        Screen::Welcome => {
            page.add(Label::new(&format!(
                "Install {} {}",
                info.name, info.version
            ))?)?;
            if let Some(publisher) = &info.publisher {
                page.add(Label::new(&format!("by {publisher}"))?)?;
            }
            if let Some(location) = &info.location {
                page.add(Label::new(&format!("into {}", location.display()))?)?;
            }
            buttons.add(button("Cancel", tx, || Message::Close)?)?;
            buttons.add(button("Install", tx, || Message::Install)?)?;
        }
        Screen::Installing => {
            page.add(Label::new(&format!("Installing {}...", info.name))?)?;
            let bar = ProgressBar::with_indeterminate(true)?;
            bar.start_animation()?;
            page.add(bar)?;
        }
        Screen::Done(done) => {
            page.add(Label::new(&format!("{} is installed.", info.name))?)?;
            buttons.add(button("Close", tx, || Message::Close)?)?;
            if done.launch.is_some() {
                buttons.add(button(&format!("Open {}", info.name), tx, || {
                    Message::Launch
                })?)?;
            }
        }
        Screen::Failed(error) => {
            page.add(Label::new("Setup failed.")?)?;
            page.add_weighted(TextView::with_content(false, error)?, 1.0)?;
            buttons.add(button("Close", tx, || Message::Close)?)?;
        }
    }
    page.add_spacer(1.0)?;
    page.add(buttons)?;
    Ok(page)
}
