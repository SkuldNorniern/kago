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
    Button, Checkbox, Container, Divider, Element, Label, Orientation, ProgressBar, Stack,
    TextField, TextView,
};
use aurea::render::{Canvas, Color, Image, Rect, RendererBackend};
use aurea::{AureaResult, Window, WindowEvent};

use super::text::{Lang, Line};
use super::wizard::{Action, Page};
use super::{Info, Session};
use crate::format::image::Rgba;

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
    let art = Art::new(&info.brand, window.scale_factor().max(1.0));
    let (content, mut live) = render(info, lang, &session, &tx, &art)?;
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
        let mut redraw = false;
        while let Ok(action) = rx.try_recv() {
            busy = true;
            if action == Action::OpenLink {
                if let Some((_, url)) = &info.brand.link {
                    Session::open_link(url);
                }
                continue;
            }
            if action == Action::Next
                && page == Page::Done
                && let Some(open) = &live.open
            {
                session.act(Action::SetOpen(open.0.borrow().get_checked()));
            }
            if page == Page::Components {
                // the boxes are read when leaving the page, either way.
                for (i, tick) in live.components.iter().enumerate() {
                    session.act(Action::SetComponent(i, tick.0.borrow().get_checked()));
                }
            }
            if page == Page::Directory {
                if let Some(field) = &live.folder {
                    let typed = field.0.borrow().get_content()?;
                    session.set_location(std::path::Path::new(typed.trim()));
                }
                if action == Action::Browse {
                    let title = lang.line(Line::DirectoryTitle);
                    session.browse(Some(window.handle() as isize), title);
                }
                // the page shows the folder and what is wrong with it.
                redraw = true;
            }
            if session.act(action) {
                return Ok(session.installed());
            }
        }
        busy |= session.pump();
        if session.wizard.page != page || redraw {
            page = session.wizard.page;
            // the old widgets go with the old content.
            drop(std::mem::take(&mut live));
            let (content, next) = render(info, lang, &session, &tx, &art)?;
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
    components: Vec<Kept<Checkbox>>,
    folder: Option<Kept<TextField>>,
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

/// the brand's images at the sizes NSIS Modern UI uses, scaled once for
/// the display.
struct Art {
    /// fitted in 150x57, top right of the inner pages.
    header: Option<Rgba>,
    /// filling 164 wide, left of the welcome and finish pages.
    side: Option<Rgba>,
    scale: f32,
}

const HEADER_IMAGE: (f32, f32) = (150.0, 57.0);
const SIDE_IMAGE: (f32, f32) = (164.0, 280.0);

impl Art {
    fn new(brand: &super::Brand, scale: f32) -> Self {
        let px = |v: f32| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let px = (v * scale).round().max(1.0) as u32;
            px
        };
        Self {
            header: brand
                .header
                .as_ref()
                .map(|h| h.fit(px(HEADER_IMAGE.0), px(HEADER_IMAGE.1))),
            side: brand
                .side
                .as_ref()
                .map(|s| s.cover(px(SIDE_IMAGE.0), px(SIDE_IMAGE.1))),
            scale,
        }
    }

    /// a canvas showing `image` at its size on screen.
    fn canvas(&self, image: &Rgba) -> AureaResult<Canvas> {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let (w, h) = (
            (image.width as f32 / self.scale).round() as u32,
            (image.height as f32 / self.scale).round() as u32,
        );
        let canvas = Canvas::new(w, h, RendererBackend::Cpu)?;
        canvas.set_background_color(Color::rgb(240, 240, 240));
        let pixels = Image::new(image.width, image.height, image.pixels.clone());
        #[allow(clippy::cast_precision_loss)]
        let dest = Rect::new(0.0, 0.0, w as f32, h as f32);
        canvas.set_draw_callback(move |ctx| ctx.draw_image_rect(&pixels, dest))?;
        Ok(canvas)
    }
}

#[allow(clippy::too_many_lines)]
fn render(
    info: &Info,
    lang: Lang,
    session: &Session,
    tx: &Sender<Action>,
    art: &Art,
) -> AureaResult<(Stack, Live)> {
    let name = info.name.as_str();
    let fill = |line| lang.fill(line, &[("name", name), ("version", &info.version)]);
    let wizard = &session.wizard;
    let mut live = Live::default();
    let mut window = Stack::new(Orientation::Vertical)?;
    let mut body = Stack::new(Orientation::Vertical)?;
    let mut buttons = Stack::new(Orientation::Horizontal)?;
    // like NSIS's BrandingText.
    buttons.add(Label::new(info.brand.text.as_deref().unwrap_or("Kago"))?)?;
    buttons.add_spacer(1.0)?;
    let header = |title: Line, subtitle: Line| -> AureaResult<Stack> {
        let mut text = Stack::new(Orientation::Vertical)?;
        text.add(Label::new(&fill(title))?)?;
        text.add(Label::new(&format!("    {}", fill(subtitle)))?)?;
        let mut header = Stack::new(Orientation::Horizontal)?;
        header.add_weighted(text, 1.0)?;
        if let Some(image) = &art.header {
            header.add(art.canvas(image)?)?;
        }
        Ok(header)
    };
    match wizard.page {
        Page::Welcome => {
            body.add(Label::new(&fill(Line::WelcomeTitle))?)?;
            let intro = info
                .brand
                .welcome
                .clone()
                .unwrap_or_else(|| fill(Line::WelcomeBody));
            body.add(Label::new(&intro)?)?;
            if let Some(publisher) = &info.publisher {
                body.add(Label::new(&lang.fill(Line::By, &[("name", publisher)]))?)?;
            }
            if let Some(location) = &info.location {
                body.add(Label::new(lang.line(Line::InstallInto))?)?;
                body.add(Label::new(&format!("    {}", location.display()))?)?;
            }
            body.add_spacer(1.0)?;
            body.add(Label::new(lang.line(Line::ClickNext))?)?;
            let next = if wizard.has_license || !info.components.is_empty() || wizard.folder_open {
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
                let next = if info.components.is_empty() && !wizard.folder_open {
                    Line::Install
                } else {
                    Line::Next
                };
                buttons.add(button(lang.line(next), tx, Action::Next)?)?;
            }
            buttons.add(button(lang.line(Line::Cancel), tx, Action::Cancel)?)?;
        }
        Page::Components => {
            window.add(header(Line::ComponentsTitle, Line::ComponentsBody)?)?;
            window.add(Divider::horizontal(WIDTH.unsigned_abs())?)?;
            for (i, (component, on)) in info.components.iter().zip(&wizard.chosen).enumerate() {
                let mut checkbox = Checkbox::with_checked(&component.name, *on)?;
                if wizard.locked(i) {
                    // like NSIS's read-only sections: shown, ticked, greyed.
                    checkbox.set_enabled(false)?;
                }
                let tick = Kept::new(checkbox);
                body.add(tick.share())?;
                live.components.push(tick);
                let (note, _) = component.note(lang);
                body.add(Label::new(&format!("        {note}"))?)?;
            }
            body.add_spacer(1.0)?;
            buttons.add(button(lang.line(Line::Back), tx, Action::Back)?)?;
            let next = if wizard.folder_open {
                Line::Next
            } else {
                Line::Install
            };
            buttons.add(button(lang.line(next), tx, Action::Next)?)?;
            buttons.add(button(lang.line(Line::Cancel), tx, Action::Cancel)?)?;
        }
        Page::Directory => {
            window.add(header(Line::DirectoryTitle, Line::DirectoryBody)?)?;
            window.add(Divider::horizontal(WIDTH.unsigned_abs())?)?;
            body.add(Label::new(&fill(Line::DirectoryHint))?)?;
            let mut row = Stack::new(Orientation::Horizontal)?;
            let shown = wizard
                .location
                .as_ref()
                .map(|l| l.display().to_string())
                .unwrap_or_default();
            let field = Kept::new(TextField::with_content(&shown)?);
            row.add_weighted(field.share(), 1.0)?;
            row.add(button(lang.line(Line::Browse), tx, Action::Browse)?)?;
            body.add(row)?;
            live.folder = Some(field);
            if let Some(error) = wizard.folder_error {
                body.add(Label::new(lang.folder_error(error))?)?;
            }
            body.add_spacer(1.0)?;
            buttons.add(button(lang.line(Line::Back), tx, Action::Back)?)?;
            buttons.add(button(lang.line(Line::Install), tx, Action::Next)?)?;
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
            let outro = info
                .brand
                .finish
                .clone()
                .unwrap_or_else(|| fill(Line::DoneBody));
            body.add(Label::new(&outro)?)?;
            if let Some((label, _)) = &info.brand.link {
                body.add(button(label, tx, Action::OpenLink)?)?;
            }
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
    // NSIS puts its welcome bitmap down the left of the first and last
    // pages.
    let framed = matches!(wizard.page, Page::Welcome | Page::Done | Page::Failed);
    match &art.side {
        Some(side) if framed => {
            let mut row = Stack::new(Orientation::Horizontal)?;
            row.add(art.canvas(side)?)?;
            row.add_weighted(body, 1.0)?;
            window.add_weighted(row, 1.0)?;
        }
        _ => window.add_weighted(body, 1.0)?,
    }
    window.add(Divider::horizontal(WIDTH.unsigned_abs())?)?;
    window.add(buttons)?;
    Ok((window, live))
}
