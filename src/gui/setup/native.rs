//! the classic look: an NSIS-style wizard of system widgets. welcome,
//! license with "I Agree", installing with a bar and a details list,
//! finish with "Run" ticked. Back, Next and Cancel sit bottom right.

use std::cell::RefCell;
use std::os::raw::c_void;
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use aurea::elements::{
    Button, Checkbox, Container, Divider, Element, Label, Orientation, ProgressBar, Stack, TextView,
};
use aurea::render::Rect;
use aurea::{AureaResult, Window, WindowEvent};

use super::text::{Lang, Line};
use super::wizard::{Action, Page};
use super::{Info, Session};

const WIDTH: i32 = 500;
const HEIGHT: i32 = 390;
/// fastest the details list is rewritten while installing.
const LOG_EVERY: Duration = Duration::from_millis(80);

pub(super) fn run(info: &Info, lang: Lang, mut session: Session) -> AureaResult<bool> {
    let mut window = Window::new(
        &lang.fill(Line::WindowTitle, &[("name", &info.name)]),
        WIDTH,
        HEIGHT,
    )?;
    let (tx, rx) = mpsc::channel();
    let mut page = session.wizard.page;
    let (content, mut live) = render(info, lang, &session, &tx)?;
    window.set_content(content)?;
    window.show();
    let mut logged = Instant::now();
    loop {
        for event in window.poll_events() {
            if matches!(event, WindowEvent::CloseRequested) && session.closable() {
                return Ok(session.installed());
            }
        }
        window.process_frames()?;
        let mut busy = false;
        while let Ok(action) = rx.try_recv() {
            busy = true;
            if action == Action::Next
                && page == Page::Done
                && let Some(open) = &live.open
            {
                session.act(Action::SetOpen(open.0.borrow().get_checked()));
            }
            if session.act(action) {
                return Ok(session.installed());
            }
        }
        busy |= session.pump();
        if session.wizard.page != page {
            page = session.wizard.page;
            // the old widgets go with the old content.
            drop(std::mem::take(&mut live));
            let (content, next) = render(info, lang, &session, &tx)?;
            window.set_content(content)?;
            live = next;
        } else if busy && page == Page::Installing {
            if let Some(bar) = &live.bar {
                bar.0
                    .borrow_mut()
                    .set_value(f64::from(session.wizard.fraction))?;
            }
            if logged.elapsed() >= LOG_EVERY
                && let Some(log) = &live.log
            {
                logged = Instant::now();
                log.0.borrow_mut().set_content(&details(lang, &session))?;
            }
        }
        if !busy {
            thread::sleep(Duration::from_millis(15));
        }
    }
}

fn details(lang: Lang, session: &Session) -> String {
    let lines: Vec<String> = session.wizard.log.iter().map(|s| s.text(lang)).collect();
    lines.join("\n")
}

/// widgets changed after the page is up.
#[derive(Default)]
struct Live {
    bar: Option<Kept<ProgressBar>>,
    log: Option<Kept<TextView>>,
    open: Option<Kept<Checkbox>>,
}

/// a widget still reachable after a container took it.
struct Kept<E>(Rc<RefCell<E>>);

impl<E> Kept<E> {
    fn new(element: E) -> Self {
        Self(Rc::new(RefCell::new(element)))
    }

    fn share(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

#[allow(
    unsafe_code,
    reason = "Element's invalidate is an unsafe trait method; this only forwards it"
)]
impl<E: Element> Element for Kept<E> {
    fn handle(&self) -> *mut c_void {
        self.0.borrow().handle()
    }

    fn released_to_parent(&self) {
        self.0.borrow().released_to_parent();
    }

    unsafe fn invalidate_platform(&self, rect: Option<Rect>) {
        // SAFETY: same element, same thread; the caller's guarantees hold.
        unsafe { self.0.borrow().invalidate_platform(rect) }
    }
}

fn button(title: &str, tx: &Sender<Action>, action: Action) -> AureaResult<Button> {
    let tx = tx.clone();
    Button::with_callback(title, move || {
        let _ = tx.send(action);
    })
}

#[allow(clippy::too_many_lines)]
fn render(
    info: &Info,
    lang: Lang,
    session: &Session,
    tx: &Sender<Action>,
) -> AureaResult<(Stack, Live)> {
    let name = info.name.as_str();
    let fill = |line| lang.fill(line, &[("name", name), ("version", &info.version)]);
    let wizard = &session.wizard;
    let mut live = Live::default();
    let mut window = Stack::new(Orientation::Vertical)?;
    let mut body = Stack::new(Orientation::Vertical)?;
    let mut buttons = Stack::new(Orientation::Horizontal)?;
    buttons.add(Label::new("Kago")?)?;
    buttons.add_spacer(1.0)?;
    let header = |title: Line, subtitle: Line| -> AureaResult<Stack> {
        let mut header = Stack::new(Orientation::Vertical)?;
        header.add(Label::new(&fill(title))?)?;
        header.add(Label::new(&format!("    {}", fill(subtitle)))?)?;
        Ok(header)
    };
    match wizard.page {
        Page::Welcome => {
            body.add(Label::new(&fill(Line::WelcomeTitle))?)?;
            body.add(Label::new(&fill(Line::WelcomeBody))?)?;
            if let Some(publisher) = &info.publisher {
                body.add(Label::new(&lang.fill(Line::By, &[("name", publisher)]))?)?;
            }
            if let Some(location) = &info.location {
                body.add(Label::new(lang.line(Line::InstallInto))?)?;
                body.add(Label::new(&format!("    {}", location.display()))?)?;
            }
            body.add_spacer(1.0)?;
            body.add(Label::new(lang.line(Line::ClickNext))?)?;
            let next = if wizard.has_license {
                Line::Next
            } else {
                Line::Install
            };
            buttons.add(button(lang.line(next), tx, Action::Next)?)?;
            buttons.add(button(lang.line(Line::Cancel), tx, Action::Cancel)?)?;
        }
        Page::License => {
            window.add(header(Line::LicenseTitle, Line::LicenseBody)?)?;
            window.add(Divider::horizontal(WIDTH.unsigned_abs())?)?;
            let text = info.license.as_ref().map_or("", |l| l.text.as_str());
            body.add_weighted(TextView::with_content(false, text)?, 1.0)?;
            if wizard.must_accept {
                body.add(Label::new(lang.line(Line::LicenseAgreeHint))?)?;
            }
            buttons.add(button(lang.line(Line::Back), tx, Action::Back)?)?;
            if wizard.must_accept {
                buttons.add(button(lang.line(Line::Agree), tx, Action::Agree)?)?;
            } else {
                buttons.add(button(lang.line(Line::Install), tx, Action::Next)?)?;
            }
            buttons.add(button(lang.line(Line::Cancel), tx, Action::Cancel)?)?;
        }
        Page::Installing => {
            window.add(header(Line::InstallingTitle, Line::InstallingBody)?)?;
            window.add(Divider::horizontal(WIDTH.unsigned_abs())?)?;
            let bar = Kept::new(ProgressBar::with_value(f64::from(wizard.fraction))?);
            body.add(bar.share())?;
            let log = Kept::new(TextView::with_content(false, &details(lang, session))?);
            body.add_weighted(log.share(), 1.0)?;
            live.bar = Some(bar);
            live.log = Some(log);
        }
        Page::Done => {
            body.add(Label::new(&fill(Line::DoneTitle))?)?;
            body.add(Label::new(&fill(Line::DoneBody))?)?;
            if wizard.launch.is_some() {
                let open = Kept::new(Checkbox::with_checked(
                    &fill(Line::RunAfter),
                    wizard.open_after,
                )?);
                body.add(open.share())?;
                live.open = Some(open);
            }
            body.add_spacer(1.0)?;
            buttons.add(button(lang.line(Line::Finish), tx, Action::Next)?)?;
        }
        Page::Failed => {
            body.add(Label::new(lang.line(Line::FailedTitle))?)?;
            body.add(Label::new(lang.line(Line::FailedBody))?)?;
            let error = wizard.error.as_deref().unwrap_or_default();
            body.add_weighted(TextView::with_content(false, error)?, 1.0)?;
            buttons.add(button(lang.line(Line::Close), tx, Action::Next)?)?;
        }
    }
    window.add_weighted(body, 1.0)?;
    window.add(Divider::horizontal(WIDTH.unsigned_abs())?)?;
    window.add(buttons)?;
    Ok((window, live))
}
