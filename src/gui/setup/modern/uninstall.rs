//! the modern look of the uninstaller: the same canvas, header and colors
//! as the setup that installed the package. asks, slides a bar while it
//! works, then shows what is gone and what stayed.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use aurea::elements::{Container, Orientation, Stack};
use aurea::render::{Canvas, Color, DrawingContext, Image, RendererBackend, request_canvas_redraw};
use aurea::{AureaResult, KeyCode, MouseButton, Window, WindowEvent};

use super::pen::{Anim, Area, Pen, color, smooth};
use super::{BUTTON_H, Frame, Hit, MARGIN, badge, buttons, caption, key, panel, title};
use crate::gui::setup::text::{Lang, Line};
use crate::gui::setup::uninstall::{Page, Removal, Removed, Removing};

const WIDTH: u32 = 600;
const HEIGHT: u32 = 360;

struct State {
    info: Removing,
    lang: Lang,
    removal: Removal,
    mouse: (f32, f32),
    pressed: Option<Hit>,
    hits: Vec<(Area, Hit)>,
    anim: Anim,
    logo: Option<Image>,
    accent: Color,
    /// when removing started, for the sliding bar.
    started: Instant,
    shown: u8,
    close: bool,
}

impl State {
    fn hovered(&self) -> Option<Hit> {
        let (x, y) = self.mouse;
        self.hits
            .iter()
            .rev()
            .find(|(a, _)| a.contains(x, y))
            .map(|(_, h)| *h)
    }

    fn click(&mut self, hit: Hit) {
        match (&self.removal.page, hit) {
            (Page::Ask, Hit::Primary) => {
                self.removal.confirm();
                self.started = Instant::now();
            }
            (Page::Ask, Hit::Secondary) | (Page::Done(_) | Page::Failed(_), Hit::Primary) => {
                self.close = true;
            }
            _ => {}
        }
    }

    /// handles one event. true when the canvas should redraw.
    fn event(&mut self, event: &WindowEvent) -> bool {
        match *event {
            WindowEvent::MouseMove { x, y, .. } => {
                let before = self.hovered();
                #[allow(clippy::cast_possible_truncation)]
                let at = (x as f32, y as f32);
                self.mouse = at;
                before != self.hovered()
            }
            WindowEvent::MouseButton {
                button: MouseButton::Left,
                pressed,
                x,
                y,
                ..
            } => {
                #[allow(clippy::cast_possible_truncation)]
                let at = (x as f32, y as f32);
                self.mouse = at;
                if pressed {
                    self.pressed = self.hovered();
                } else if let Some(hit) = self.pressed.take()
                    && Some(hit) == self.hovered()
                {
                    self.click(hit);
                }
                true
            }
            WindowEvent::MouseExited => {
                self.mouse = (-1.0, -1.0);
                true
            }
            WindowEvent::KeyInput {
                key, pressed: true, ..
            } => {
                match key {
                    KeyCode::Enter => self.click(Hit::Primary),
                    KeyCode::Escape if self.removal.closable() => self.close = true,
                    _ => return false,
                }
                true
            }
            _ => false,
        }
    }
}

/// shows the modern uninstaller until it is closed. returns whether the
/// package was removed.
pub(in crate::gui::setup) fn run(
    info: &Removing,
    lang: Lang,
    removal: Removal,
) -> AureaResult<bool> {
    let mut window = Window::new(
        &lang.fill(Line::UninstallTitle, &[("name", &info.name)]),
        WIDTH.cast_signed(),
        HEIGHT.cast_signed(),
    )?;
    let canvas = Canvas::new(WIDTH, HEIGHT, RendererBackend::Cpu)?;
    canvas.set_background_color(color::BACKGROUND);
    let scale = window.scale_factor().max(1.0);
    let logo = info.logo.as_ref().map(|l| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let px = (52.0 * scale).round() as u32;
        let fitted = l.fit(px, px);
        Image::new(fitted.width, fitted.height, fitted.pixels)
    });
    let state = Rc::new(RefCell::new(State {
        info: info.clone(),
        lang,
        removal,
        mouse: (-1.0, -1.0),
        pressed: None,
        hits: Vec::new(),
        anim: Anim::default(),
        logo,
        accent: info
            .accent
            .map_or(color::ACCENT, |[r, g, b]| Color::rgb(r, g, b)),
        started: Instant::now(),
        shown: 0,
        close: false,
    }));
    let drawn = Rc::clone(&state);
    canvas.set_draw_callback(move |ctx| draw(ctx, &mut drawn.borrow_mut()))?;
    let id = canvas.id();
    let mut layout = Stack::new(Orientation::Vertical)?;
    layout.add(canvas)?;
    window.set_content(layout)?;
    window.show();
    request_canvas_redraw(id);
    loop {
        let mut redraw = false;
        for event in window.wait_events(Some(super::super::PUMP)) {
            let mut s = state.borrow_mut();
            if matches!(event, WindowEvent::CloseRequested) && s.removal.closable() {
                return Ok(s.removal.removed());
            }
            redraw |= s.event(&event) || matches!(event, WindowEvent::Resized { .. });
        }
        {
            let mut s = state.borrow_mut();
            redraw |= s.removal.pump();
            if s.close && s.removal.closable() {
                return Ok(s.removal.removed());
            }
            // the bar slides for as long as removing takes.
            redraw |= s.anim.busy() || matches!(s.removal.page, Page::Working);
        }
        if redraw {
            request_canvas_redraw(id);
        }
    }
}

fn draw(ctx: &mut dyn DrawingContext, s: &mut State) -> AureaResult<()> {
    #[allow(clippy::cast_precision_loss)]
    let (w, h) = (ctx.width() as f32, ctx.height() as f32);
    let mut anim = std::mem::take(&mut s.anim);
    anim.frame();
    let page = match s.removal.page {
        Page::Ask => 1,
        Page::Working => 2,
        Page::Done(_) => 3,
        Page::Failed(_) => 4,
    };
    if s.shown != page {
        if s.shown != 0 {
            anim.set(key::PAGE, 0.0);
        }
        s.shown = page;
    }
    let mut pen = Pen {
        ctx,
        lang: s.lang,
        accent: s.accent,
        on_accent: super::pen::on(s.accent),
    };
    let mut frame = Frame {
        hits: Vec::new(),
        hovered: s.hovered(),
        pressed: s.pressed,
        anim: &mut anim,
    };
    pen.fill(Area::new(0.0, 0.0, w, h), color::BACKGROUND)?;
    title(
        &mut pen,
        (s.logo.as_ref(), None),
        (&s.info.name, &s.info.version),
        0.0,
        w,
    )?;
    let arrived = smooth(frame.anim.to(key::PAGE, 1.0, 10.0));
    let rise = 12.0 * (1.0 - arrived);
    let inner = w - 2.0 * MARGIN;
    let body = Area::new(
        MARGIN,
        112.0 + rise,
        inner,
        h - 112.0 - MARGIN - BUTTON_H - 20.0,
    );
    let footer = Area::new(MARGIN, h - MARGIN - BUTTON_H + rise, inner, BUTTON_H);
    if let Some(text) = &s.info.text {
        pen.clipped(
            text,
            Area::new(footer.x, footer.y, (footer.w - 280.0).max(0.0), footer.h),
            &pen.font(12.0),
            color::FAINT,
        )?;
    }
    match &s.removal.page {
        Page::Ask => ask(&mut pen, &mut frame, s, body, footer)?,
        Page::Working => working(&mut pen, s, body)?,
        Page::Done(done) => finished(&mut pen, &mut frame, s, (body, footer), done)?,
        Page::Failed(error) => failed(&mut pen, &mut frame, s, (body, footer), error)?,
    }
    if arrived < 1.0 {
        pen.veil(
            Area::new(0.0, 100.0, w, h - 100.0),
            color::BACKGROUND,
            1.0 - arrived,
        )?;
    }
    s.hits = frame.hits;
    s.anim = anim;
    Ok(())
}

/// `line` with the package's name and version.
fn fill(s: &State, line: Line) -> String {
    s.lang.fill(
        line,
        &[("name", s.info.name.as_str()), ("version", &s.info.version)],
    )
}

/// asks first, with where the app is.
fn ask(
    pen: &mut Pen<'_>,
    frame: &mut Frame<'_>,
    s: &State,
    body: Area,
    footer: Area,
) -> AureaResult<()> {
    let lang = s.lang;
    let fill = |line| fill(s, line);
    let used = pen.wrapped(
        &fill(Line::UninstallAsk),
        Area::new(body.x, body.y, body.w, 44.0),
        &pen.font(14.0),
        color::TEXT,
    )?;
    if let Some(location) = &s.info.location {
        let card = Area::new(body.x, body.y + used + 14.0, body.w, 64.0);
        panel(pen, card)?;
        caption(
            pen,
            lang.line(Line::InstalledIn),
            card.x + 16.0,
            card.y + 12.0,
        )?;
        pen.clipped(
            &location.display().to_string(),
            Area::new(card.x + 16.0, card.y + 32.0, card.w - 32.0, 20.0),
            &pen.font(14.0),
            color::TEXT,
        )?;
    }
    if let Some(note) = s.info.running_note(lang) {
        let y = body.y + used + 14.0 + if s.info.location.is_some() { 76.0 } else { 0.0 };
        pen.wrapped(
            &note,
            Area::new(body.x, y, body.w, 40.0),
            &pen.font(13.0),
            color::WARN,
        )?;
    }
    buttons(
        pen,
        frame,
        footer,
        (lang.line(Line::Uninstall), true),
        Some(lang.line(Line::Cancel)),
    )?;
    Ok(())
}

/// removing. no steps to count: a short piece runs along the track.
fn working(pen: &mut Pen<'_>, s: &State, body: Area) -> AureaResult<()> {
    let fill = |line| fill(s, line);
    pen.text(
        &fill(Line::Removing),
        body.x,
        body.y + 8.0,
        &pen.bold(18.0),
        color::TEXT,
    )?;
    let track = Area::new(body.x, body.y + 52.0, body.w, 8.0);
    pen.round(track, 4.0, color::RAISED)?;
    let t = (s.started.elapsed().as_secs_f32() / 1.4).fract();
    let piece = track.w * 0.3;
    let x = track.x - piece + (track.w + piece) * smooth(t);
    let from = x.max(track.x);
    let to = (x + piece).min(track.right());
    if to > from {
        pen.round(
            Area::new(from, track.y, to - from, track.h),
            4.0,
            pen.accent,
        )?;
    }
    Ok(())
}

/// gone, with what stayed.
fn finished(
    pen: &mut Pen<'_>,
    frame: &mut Frame<'_>,
    s: &State,
    (body, footer): (Area, Area),
    done: &Removed,
) -> AureaResult<()> {
    let lang = s.lang;
    let fill = |line| fill(s, line);
    let whole = done.failed.is_empty();
    badge(pen, body.x + 22.0, body.y + 22.0, whole)?;
    pen.clipped(
        &fill(done.headline()),
        Area::new(body.x + 60.0, body.y + 10.0, body.w - 60.0, 24.0),
        &pen.bold(16.0),
        color::TEXT,
    )?;
    let notes = Removal::notes(lang, done);
    if !notes.is_empty() {
        let well = Area::new(body.x, body.y + 60.0, body.w, body.h - 60.0);
        panel(pen, well)?;
        let font = pen.font(13.0);
        let inner = well.inset(14.0);
        for (i, note) in notes.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let y = inner.y + i as f32 * 19.0;
            if y + 19.0 > inner.bottom() {
                break;
            }
            pen.clipped(
                note,
                Area::new(inner.x, y, inner.w, 18.0),
                &font,
                color::DIM,
            )?;
        }
    }
    buttons(pen, frame, footer, (lang.line(Line::Close), true), None)?;
    Ok(())
}

fn failed(
    pen: &mut Pen<'_>,
    frame: &mut Frame<'_>,
    s: &State,
    (body, footer): (Area, Area),
    error: &str,
) -> AureaResult<()> {
    let lang = s.lang;
    badge(pen, body.x + 22.0, body.y + 22.0, false)?;
    pen.clipped(
        lang.line(Line::UninstallFailed),
        Area::new(body.x + 60.0, body.y + 10.0, body.w - 60.0, 24.0),
        &pen.bold(16.0),
        color::TEXT,
    )?;
    let well = Area::new(body.x, body.y + 60.0, body.w, body.h - 60.0);
    panel(pen, well)?;
    pen.wrapped(error, well.inset(14.0), &pen.font(13.0), color::BAD)?;
    buttons(pen, frame, footer, (lang.line(Line::Close), true), None)?;
    Ok(())
}
