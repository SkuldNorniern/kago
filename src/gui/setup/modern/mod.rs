//! the modern look: one Aurea canvas, drawn like Lykil and Aira Studio.
//! dark panels, an accent pill for the main action, pages that rise in,
//! a progress bar that eases to where the install is.

mod pen;

use std::cell::RefCell;
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use aurea::elements::{Container, Orientation, Stack};
use aurea::render::{Canvas, DrawingContext, RendererBackend, request_canvas_redraw};
use aurea::{AureaResult, KeyCode, MouseButton, Window, WindowEvent};

use super::text::{Lang, Line};
use super::wizard::{Action, Page};
use super::{Info, Session};
use aurea::render::{Color, Image};
use pen::{Anim, Area, Pen, color, smooth};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 420;
/// a components caption and one row per component.
const COMPONENTS_CAPTION: f32 = 24.0;
const COMPONENT_ROW: f32 = 30.0;
const MARGIN: f32 = 32.0;
const BUTTON_H: f32 = 40.0;

/// eased value keys.
mod key {
    pub const PAGE: u32 = 1;
    pub const PROGRESS: u32 = 2;
    pub const ACCEPT: u32 = 3;
    pub const SCROLL: u32 = 4;

    pub const fn hover(hit: super::Hit) -> u32 {
        match hit {
            super::Hit::Primary => 100,
            super::Hit::Secondary => 101,
            super::Hit::Accept => 102,
            super::Hit::License => 103,
            super::Hit::Browse => 104,
            super::Hit::Link => 105,
            super::Hit::Component(i) => 200 + i,
        }
    }

    pub const fn ticked(component: u32) -> u32 {
        400 + component
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Primary,
    Secondary,
    Accept,
    License,
    /// the optional component at this index.
    Component(u32),
    /// change the install folder.
    Browse,
    /// the brand link on the finish page.
    Link,
}

struct State {
    info: Info,
    lang: Lang,
    session: Session,
    mouse: (f32, f32),
    pressed: Option<Hit>,
    hits: Vec<(Area, Hit)>,
    anim: Anim,
    /// page drawn last frame, to start the rise when it changes.
    shown: Option<Page>,
    scroll: f32,
    scroll_max: f32,
    close: bool,
    /// the folder dialog was asked for; the loop opens it.
    want_browse: bool,
    art: Art,
}

/// the brand's images, scaled once for the display they are shown on.
#[derive(Default)]
struct Art {
    logo: Option<Image>,
    header: Option<Image>,
    /// already cropped to fill the side panel.
    side: Option<Image>,
    accent: Option<Color>,
}

/// width of the side panel when the brand has a side image.
const SIDE_W: f32 = 180.0;

impl Art {
    fn new(brand: &super::Brand, height: f32, scale: f32) -> Self {
        let px = |v: f32| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let px = (v * scale).round().max(1.0) as u32;
            px
        };
        let image =
            |rgba: crate::format::image::Rgba| Image::new(rgba.width, rgba.height, rgba.pixels);
        Self {
            logo: brand
                .logo
                .as_ref()
                .map(|l| image(l.fit(px(52.0), px(52.0)))),
            header: brand
                .header
                .as_ref()
                .map(|h| image(h.fit(px(150.0), px(57.0)))),
            side: brand
                .side
                .as_ref()
                .map(|s| image(s.cover(px(SIDE_W), px(height)))),
            accent: brand.accent.map(|[r, g, b]| Color::rgb(r, g, b)),
        }
    }
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
        let wizard = &self.session.wizard;
        let action = match (wizard.page, hit) {
            (Page::Welcome | Page::License, Hit::Primary) => Action::Install,
            (Page::Welcome, Hit::Secondary) | (Page::Failed, _) => Action::Cancel,
            (Page::Welcome, Hit::License) | (Page::Done, Hit::Primary) => Action::Next,
            (Page::License, Hit::Secondary) => Action::Back,
            (_, Hit::Accept) => Action::ToggleAccept,
            (_, Hit::Browse) => {
                self.want_browse = true;
                return;
            }
            (_, Hit::Link) => {
                if let Some((_, url)) = &self.info.brand.link {
                    Session::open_link(url);
                }
                return;
            }
            (_, Hit::Component(i)) => {
                let i = i as usize;
                let on = wizard.chosen.get(i).is_some_and(|on| *on);
                Action::SetComponent(i, !on)
            }
            (Page::Done, Hit::Secondary) => {
                self.session.act(Action::SetOpen(false));
                Action::Next
            }
            _ => return,
        };
        self.close |= self.session.act(action);
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
            WindowEvent::MouseWheel { delta_y, .. }
                if self.session.wizard.page == Page::License =>
            {
                #[allow(clippy::cast_possible_truncation)]
                let step = delta_y as f32 * 48.0;
                self.scroll = (self.scroll - step).clamp(0.0, self.scroll_max);
                true
            }
            WindowEvent::KeyInput {
                key, pressed: true, ..
            } => {
                match key {
                    KeyCode::Enter => self.click(Hit::Primary),
                    KeyCode::Escape if self.session.closable() => {
                        self.close |= self.session.act(Action::Cancel);
                    }
                    KeyCode::Space if self.session.wizard.page == Page::License => {
                        self.click(Hit::Accept);
                    }
                    _ => return false,
                }
                true
            }
            _ => false,
        }
    }
}

pub(super) fn run(info: &Info, lang: Lang, session: Session) -> AureaResult<bool> {
    // the welcome page grows by the component rows it lists.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let height = if info.components.is_empty() {
        HEIGHT
    } else {
        HEIGHT + (COMPONENTS_CAPTION + COMPONENT_ROW * info.components.len() as f32).ceil() as u32
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let width = if info.brand.side.is_some() {
        WIDTH + SIDE_W as u32
    } else {
        WIDTH
    };
    let mut window = Window::new(
        &lang.fill(Line::WindowTitle, &[("name", &info.name)]),
        width.cast_signed(),
        height.cast_signed(),
    )?;
    let canvas = Canvas::new(width, height, RendererBackend::Cpu)?;
    #[allow(clippy::cast_precision_loss)]
    let art = Art::new(&info.brand, height as f32, window.scale_factor().max(1.0));
    canvas.set_background_color(color::BACKGROUND);
    let state = Rc::new(RefCell::new(State {
        info: info.clone(),
        lang,
        session,
        mouse: (-1.0, -1.0),
        pressed: None,
        hits: Vec::new(),
        anim: Anim::default(),
        shown: None,
        scroll: 0.0,
        scroll_max: 0.0,
        close: false,
        want_browse: false,
        art,
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
        for event in window.poll_events() {
            let mut s = state.borrow_mut();
            if matches!(event, WindowEvent::CloseRequested) && s.session.closable() {
                return Ok(s.session.installed());
            }
            redraw |= s.event(&event) || matches!(event, WindowEvent::Resized { .. });
        }
        let ask = {
            let mut s = state.borrow_mut();
            redraw |= s.session.pump();
            if s.close {
                return Ok(s.session.installed());
            }
            redraw |= s.anim.busy();
            if std::mem::take(&mut s.want_browse) {
                s.session.browser()
            } else {
                None
            }
        };
        // the dialog runs its own message loop, which repaints this
        // window, so the state must not be borrowed while it is open.
        if let Some((browse, start)) = ask {
            let title = lang.line(Line::DirectoryTitle);
            let owner = window.handle() as isize;
            if let Some(dir) = browse(Some(owner), title, start.as_deref()) {
                state.borrow_mut().session.picked(&dir);
            }
            redraw = true;
        }
        if redraw {
            request_canvas_redraw(id);
        }
        window.process_frames()?;
        if !redraw {
            thread::sleep(Duration::from_millis(15));
        }
    }
}

fn draw(ctx: &mut dyn DrawingContext, s: &mut State) -> AureaResult<()> {
    #[allow(clippy::cast_precision_loss)]
    let (w, h) = (ctx.width() as f32, ctx.height() as f32);
    let mut anim = std::mem::take(&mut s.anim);
    anim.frame();
    let page = s.session.wizard.page;
    if s.shown != Some(page) {
        if s.shown.is_some() {
            anim.set(key::PAGE, 0.0);
        }
        s.shown = Some(page);
        s.scroll = 0.0;
    }
    let accent = s.art.accent.unwrap_or(color::ACCENT);
    let mut pen = Pen {
        ctx,
        lang: s.lang,
        accent,
        on_accent: pen::on(accent),
    };
    let mut frame = Frame {
        hits: Vec::new(),
        hovered: s.hovered(),
        pressed: s.pressed,
        anim: &mut anim,
    };
    pen.fill(Area::new(0.0, 0.0, w, h), color::BACKGROUND)?;
    // the side panel, like NSIS's welcome bitmap, takes the left edge.
    let left = if let Some(side) = &s.art.side {
        pen.ctx
            .draw_image_rect(side, aurea::render::Rect::new(0.0, 0.0, SIDE_W, h))?;
        SIDE_W
    } else {
        0.0
    };
    header(&mut pen, s, left, w)?;

    let arrived = smooth(frame.anim.to(key::PAGE, 1.0, 10.0));
    let rise = 12.0 * (1.0 - arrived);
    let inner = w - left - 2.0 * MARGIN;
    let body = Area::new(
        left + MARGIN,
        112.0 + rise,
        inner,
        h - 112.0 - MARGIN - BUTTON_H - 20.0,
    );
    let footer = Area::new(left + MARGIN, h - MARGIN - BUTTON_H + rise, inner, BUTTON_H);
    if let Some(text) = &s.info.brand.text {
        // NSIS's branding text, quiet at the bottom left.
        pen.clipped(
            text,
            Area::new(footer.x, footer.y, (footer.w - 280.0).max(0.0), footer.h),
            &pen.font(12.0),
            color::FAINT,
        )?;
    }
    match page {
        Page::Welcome | Page::Components | Page::Directory => {
            welcome(&mut pen, &mut frame, s, body, footer)?;
        }
        Page::License => license(&mut pen, &mut frame, s, body, footer)?,
        Page::Installing => installing(&mut pen, &mut frame, s, body)?,
        Page::Done => done(&mut pen, &mut frame, s, body, footer)?,
        Page::Failed => failed(&mut pen, &mut frame, s, body, footer)?,
    }
    if arrived < 1.0 {
        pen.veil(
            Area::new(left, 100.0, w - left, h - 100.0),
            color::BACKGROUND,
            1.0 - arrived,
        )?;
    }
    s.hits = frame.hits;
    s.anim = anim;
    Ok(())
}

/// per-frame input and the hits drawn this frame.
struct Frame<'a> {
    hits: Vec<(Area, Hit)>,
    hovered: Option<Hit>,
    pressed: Option<Hit>,
    anim: &'a mut Anim,
}

impl Frame<'_> {
    fn hover(&mut self, hit: Hit) -> f32 {
        let target = f32::from(self.hovered == Some(hit));
        self.anim.to(key::hover(hit), target, 18.0)
    }

    fn pressed(&self, hit: Hit) -> bool {
        self.pressed == Some(hit) && self.hovered == Some(hit)
    }
}

/// the app's mark, name and version, the same on every page. the mark is
/// the brand logo or app icon when there is one, else the first letter.
#[allow(
    clippy::cast_precision_loss,
    reason = "image sides are a few hundred pixels"
)]
fn header(pen: &mut Pen<'_>, s: &State, left: f32, w: f32) -> AureaResult<()> {
    let info = &s.info;
    let mark = Area::new(left + MARGIN, MARGIN, 52.0, 52.0);
    if let Some(logo) = &s.art.logo {
        // fitted in 52x52 keeping its shape: center the shorter side.
        let scale = 52.0 / logo.width.max(logo.height) as f32;
        let (lw, lh) = (logo.width as f32 * scale, logo.height as f32 * scale);
        let at = aurea::render::Rect::new(
            mark.x + (52.0 - lw) / 2.0,
            mark.y + (52.0 - lh) / 2.0,
            lw,
            lh,
        );
        pen.ctx.draw_image_rect(logo, at)?;
    } else {
        pen.round(mark, 14.0, pen.accent)?;
        let initial: String = info.name.chars().take(1).collect();
        pen.centred(
            &initial.to_uppercase(),
            mark,
            &pen.bold(24.0),
            pen.on_accent,
        )?;
    }
    let mut right = w - MARGIN;
    if let Some(image) = &s.art.header {
        let scale = (150.0 / image.width as f32).min(57.0 / image.height as f32);
        let (iw, ih) = (image.width as f32 * scale, image.height as f32 * scale);
        right -= iw;
        let at = aurea::render::Rect::new(right, MARGIN + (52.0 - ih) / 2.0, iw, ih);
        pen.ctx.draw_image_rect(image, at)?;
        right -= 16.0;
    }
    let x = mark.right() + 16.0;
    let text_w = (right - x).max(0.0);
    pen.clipped(
        &info.name,
        Area::new(x, MARGIN + 2.0, text_w, 28.0),
        &pen.bold(22.0),
        color::TEXT,
    )?;
    let mut sub = info.version.clone();
    if let Some(publisher) = &info.publisher {
        sub = format!("{sub}  \u{b7}  {publisher}");
    }
    pen.clipped(
        &sub,
        Area::new(x, MARGIN + 32.0, text_w, 18.0),
        &pen.font(13.0),
        color::DIM,
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Main,
    Quiet,
    Off,
}

fn pill(
    pen: &mut Pen<'_>,
    frame: &mut Frame<'_>,
    a: Area,
    text: &str,
    hit: Hit,
    tone: Tone,
) -> AureaResult<()> {
    let t = if tone == Tone::Off {
        0.0
    } else {
        frame.hover(hit)
    };
    let (bg, fg) = match tone {
        Tone::Main => (color::mix(pen.accent, color::TEXT, t * 0.18), pen.on_accent),
        Tone::Quiet => (
            color::mix(color::RAISED, color::HOVER, t),
            color::mix(color::DIM, color::TEXT, t),
        ),
        Tone::Off => (color::SURFACE, color::FAINT),
    };
    let bg = if frame.pressed(hit) {
        color::mix(bg, color::BACKGROUND, 0.25)
    } else {
        bg
    };
    pen.round(a, 10.0, bg)?;
    if tone != Tone::Main {
        pen.outline(a, 10.0, 1.0, color::mix(color::BORDER, color::DIM, t * 0.4))?;
    }
    let font = pen.bold(14.0);
    pen.centred(text, a, &font, fg)?;
    if tone != Tone::Off {
        frame.hits.push((a, hit));
    }
    Ok(())
}

/// main action at the right of the footer, an optional second one left of it.
fn buttons(
    pen: &mut Pen<'_>,
    frame: &mut Frame<'_>,
    footer: Area,
    main: (&str, bool),
    second: Option<&str>,
) -> AureaResult<()> {
    let main_w = (pen.width(main.0, &pen.bold(14.0)) + 48.0).max(128.0);
    let a = Area::new(footer.right() - main_w, footer.y, main_w, footer.h);
    let tone = if main.1 { Tone::Main } else { Tone::Off };
    pill(pen, frame, a, main.0, Hit::Primary, tone)?;
    if let Some(second) = second {
        let w = (pen.width(second, &pen.bold(14.0)) + 40.0).max(104.0);
        let b = Area::new(a.x - 12.0 - w, footer.y, w, footer.h);
        pill(pen, frame, b, second, Hit::Secondary, Tone::Quiet)?;
    }
    Ok(())
}

fn panel(pen: &mut Pen<'_>, a: Area) -> AureaResult<()> {
    pen.round(a, 12.0, color::SURFACE)?;
    pen.outline(a, 12.0, 1.0, color::mix(color::SURFACE, color::BORDER, 0.6))
}

fn caption(pen: &mut Pen<'_>, text: &str, x: f32, y: f32) -> AureaResult<()> {
    let font = pen.bold(12.0);
    pen.text(text, x, y, &font, color::DIM)
}

/// a tick box with its label; returns the area it took.
fn tick(
    pen: &mut Pen<'_>,
    frame: &mut Frame<'_>,
    at: (f32, f32),
    (text, hit): (&str, Option<Hit>),
    on: f32,
) -> AureaResult<Area> {
    let font = pen.font(14.0);
    let w = 22.0 + 10.0 + pen.width(text, &font);
    let a = Area::new(at.0, at.1, w, 24.0);
    // no hit: locked, ticked and quieter.
    let t = hit.map_or(0.0, |hit| frame.hover(hit));
    let fill = if hit.is_some() {
        pen.accent
    } else {
        color::mix(pen.accent, color::SURFACE, 0.45)
    };
    let boxed = Area::new(a.x, a.y + 2.0, 20.0, 20.0);
    pen.round(
        boxed,
        6.0,
        color::mix(color::mix(color::RAISED, color::HOVER, t), fill, on),
    )?;
    pen.outline(boxed, 6.0, 1.0, color::mix(color::BORDER, fill, on))?;
    if on > 0.05 {
        let c = color::mix(color::RAISED, pen.on_accent, on);
        pen.line(
            (boxed.x + 5.0, boxed.y + 10.5),
            (boxed.x + 8.5, boxed.y + 14.0),
            2.0,
            c,
        )?;
        pen.line(
            (boxed.x + 8.5, boxed.y + 14.0),
            (boxed.x + 15.0, boxed.y + 6.5),
            2.0,
            c,
        )?;
    }
    pen.clipped(
        text,
        Area::new(boxed.right() + 10.0, a.y, w - 32.0, a.h),
        &font,
        color::mix(color::DIM, color::TEXT, t.max(on)),
    )?;
    if let Some(hit) = hit {
        frame.hits.push((a, hit));
    }
    Ok(a)
}

/// where the install goes, with Change when the folder is open and a
/// note when it is an update or cannot be used. returns its bottom edge.
fn location_card(
    pen: &mut Pen<'_>,
    frame: &mut Frame<'_>,
    s: &State,
    location: &std::path::Path,
    at: Area,
) -> AureaResult<f32> {
    let lang = s.lang;
    let wizard = &s.session.wizard;
    let font = pen.font(14.0);
    let body = at;
    let y = at.y;
    let note = if let Some(error) = wizard.folder_error {
        Some((lang.folder_error(error), color::BAD))
    } else if s.info.upgrade {
        Some((lang.line(Line::UpdateHere), color::FAINT))
    } else {
        None
    };
    let card_h = if note.is_some() { 84.0 } else { 64.0 };
    let card = Area::new(body.x, y, body.w, card_h);
    panel(pen, card)?;
    caption(
        pen,
        lang.line(Line::InstallInto),
        card.x + 16.0,
        card.y + 12.0,
    )?;
    let mut text_w = card.w - 32.0;
    if wizard.folder_open {
        let label = lang.line(Line::Change);
        let w = (pen.width(label, &pen.bold(13.0)) + 28.0).max(80.0);
        let change = Area::new(card.right() - 16.0 - w, card.y + 16.0, w, 32.0);
        pill(pen, frame, change, label, Hit::Browse, Tone::Quiet)?;
        text_w -= w + 12.0;
    }
    pen.clipped(
        &location.display().to_string(),
        Area::new(card.x + 16.0, card.y + 32.0, text_w, 20.0),
        &font,
        color::TEXT,
    )?;
    if let Some((text, tone)) = note {
        pen.clipped(
            text,
            Area::new(card.x + 16.0, card.y + 56.0, card.w - 32.0, 18.0),
            &pen.font(12.0),
            tone,
        )?;
    }
    Ok(card.bottom())
}

fn welcome(
    pen: &mut Pen<'_>,
    frame: &mut Frame<'_>,
    s: &State,
    body: Area,
    footer: Area,
) -> AureaResult<()> {
    let lang = s.lang;
    let fill = |line| {
        lang.fill(
            line,
            &[("name", &s.info.name), ("version", &s.info.version)],
        )
    };
    let font = pen.font(14.0);
    let intro = s
        .info
        .brand
        .welcome
        .clone()
        .unwrap_or_else(|| fill(Line::WelcomeBody));
    let used = pen.wrapped(
        &intro,
        Area::new(body.x, body.y, body.w, 44.0),
        &font,
        color::TEXT,
    )?;
    let mut y = body.y + used + 12.0;
    let wizard = &s.session.wizard;
    if let Some(location) = &wizard.location {
        y = location_card(pen, frame, s, location, Area::new(body.x, y, body.w, 0.0))? + 16.0;
    }
    if !s.info.components.is_empty() {
        caption(pen, lang.line(Line::Components), body.x, y)?;
        y += COMPONENTS_CAPTION;
        for (i, component) in s.info.components.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let index = i as u32;
            let picked = wizard.chosen.get(i).is_some_and(|on| *on);
            let on = frame.anim.to(key::ticked(index), f32::from(picked), 16.0);
            let row = tick(
                pen,
                frame,
                (body.x, y),
                (
                    &component.name,
                    (!wizard.locked(i)).then_some(Hit::Component(index)),
                ),
                on,
            )?;
            let (note, warn) = component.note(lang);
            let left = row.right() + 12.0;
            pen.clipped(
                &note,
                Area::new(left, y, (body.right() - left).max(0.0), 24.0),
                &pen.font(13.0),
                if warn { color::WARN } else { color::FAINT },
            )?;
            y += COMPONENT_ROW;
        }
        y += 6.0;
    }
    if wizard.has_license {
        let mut left = body.x;
        if wizard.must_accept {
            let on = frame.anim.to(key::ACCEPT, f32::from(wizard.accepted), 16.0);
            let ticked = tick(
                pen,
                frame,
                (left, y),
                (lang.line(Line::LicenseAccept), Some(Hit::Accept)),
                on,
            )?;
            left = ticked.right() + 16.0;
        }
        let link = lang.line(Line::LicenseRead);
        let link_font = pen.bold(14.0);
        let link_w = pen.width(link, &link_font);
        let link_area = Area::new(left, y, link_w, 24.0);
        let hover = frame.hover(Hit::License);
        let tone = color::mix(pen.accent, color::TEXT, 0.35 + hover * 0.4);
        pen.clipped(link, link_area, &link_font, tone)?;
        frame.hits.push((link_area, Hit::License));
    }
    // with the license not yet accepted, Install opens it.
    buttons(
        pen,
        frame,
        footer,
        (lang.line(Line::Install), wizard.folder_error.is_none()),
        Some(lang.line(Line::Cancel)),
    )
}

fn license(
    pen: &mut Pen<'_>,
    frame: &mut Frame<'_>,
    s: &mut State,
    body: Area,
    footer: Area,
) -> AureaResult<()> {
    let lang = s.lang;
    let wizard = &s.session.wizard;
    let tick_h = if wizard.must_accept { 36.0 } else { 0.0 };
    let card = Area::new(body.x, body.y - 8.0, body.w, body.h + 8.0 - tick_h);
    panel(pen, card)?;
    let inner = card.inset(16.0);
    let font = pen.font(13.0);
    let line_h = font.size + 6.0;
    let text = s.info.license.as_ref().map_or("", |l| l.text.as_str());
    let lines = pen.lines(text, inner.w - 8.0, &font);
    #[allow(clippy::cast_precision_loss)]
    let total = lines.len() as f32 * line_h;
    s.scroll_max = (total - inner.h).max(0.0);
    s.scroll = s.scroll.min(s.scroll_max);
    let scroll = frame.anim.to(key::SCROLL, s.scroll, 20.0);
    for (i, line) in lines.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let y = inner.y + i as f32 * line_h - scroll;
        // whole lines only: nothing spills past the card.
        if y < inner.y - 0.5 || y + line_h > inner.bottom() + 0.5 {
            continue;
        }
        pen.text(line, inner.x, y, &font, color::DIM)?;
    }
    if s.scroll_max > 0.0 {
        let track = Area::new(card.right() - 8.0, inner.y, 3.0, inner.h);
        let thumb_h = (inner.h * inner.h / total).max(24.0);
        let thumb_y = track.y + (track.h - thumb_h) * (scroll / s.scroll_max);
        pen.round(
            Area::new(track.x, thumb_y, track.w, thumb_h),
            1.5,
            color::BORDER,
        )?;
    }
    if wizard.must_accept {
        let on = frame.anim.to(key::ACCEPT, f32::from(wizard.accepted), 16.0);
        tick(
            pen,
            frame,
            (body.x, card.bottom() + 12.0),
            (lang.line(Line::LicenseAccept), Some(Hit::Accept)),
            on,
        )?;
    }
    buttons(
        pen,
        frame,
        footer,
        (lang.line(Line::Install), wizard.can_go_on()),
        Some(lang.line(Line::Return)),
    )
}

fn installing(pen: &mut Pen<'_>, frame: &mut Frame<'_>, s: &State, body: Area) -> AureaResult<()> {
    let lang = s.lang;
    let wizard = &s.session.wizard;
    let title = pen.bold(18.0);
    pen.text(
        lang.line(Line::InstallingTitle),
        body.x,
        body.y + 8.0,
        &title,
        color::TEXT,
    )?;
    let shown = frame.anim.to(key::PROGRESS, wizard.fraction, 8.0);
    let track = Area::new(body.x, body.y + 52.0, body.w, 8.0);
    pen.round(track, 4.0, color::RAISED)?;
    if shown > 0.0 {
        pen.round(
            Area::new(track.x, track.y, (track.w * shown).max(8.0), track.h),
            4.0,
            pen.accent,
        )?;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let percent = format!("{}%", (shown * 100.0).round() as u32);
    let small = pen.font(13.0);
    let pw = pen.width(&percent, &small);
    pen.text(
        &percent,
        track.right() - pw,
        track.bottom() + 12.0,
        &small,
        color::DIM,
    )?;
    if let Some(step) = wizard.log.last() {
        pen.clipped(
            &step.text(lang),
            Area::new(track.x, track.bottom() + 10.0, track.w - pw - 16.0, 18.0),
            &small,
            color::FAINT,
        )?;
    }
    // the last few steps, fading upward.
    let steps = wizard.log.iter().rev().skip(1).take(4);
    for (i, step) in steps.enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let i = i as f32;
        let c = color::mix(color::FAINT, color::BACKGROUND, 0.25 + i * 0.18);
        pen.clipped(
            &step.text(lang),
            Area::new(track.x, track.bottom() + 34.0 + i * 20.0, track.w, 18.0),
            &small,
            c,
        )?;
    }
    Ok(())
}

/// a round badge with a tick or a cross.
fn badge(pen: &mut Pen<'_>, x: f32, y: f32, good: bool) -> AureaResult<()> {
    let c = if good { color::GOOD } else { color::BAD };
    pen.circle(x, y, 22.0, color::mix(color::BACKGROUND, c, 0.22))?;
    pen.circle(x, y, 16.0, c)?;
    if good {
        pen.line(
            (x - 7.0, y + 0.5),
            (x - 2.0, y + 5.5),
            2.5,
            color::ACCENT_TEXT,
        )?;
        pen.line(
            (x - 2.0, y + 5.5),
            (x + 7.5, y - 5.0),
            2.5,
            color::ACCENT_TEXT,
        )
    } else {
        pen.line(
            (x - 5.5, y - 5.5),
            (x + 5.5, y + 5.5),
            2.5,
            color::ACCENT_TEXT,
        )?;
        pen.line(
            (x + 5.5, y - 5.5),
            (x - 5.5, y + 5.5),
            2.5,
            color::ACCENT_TEXT,
        )
    }
}

fn done(
    pen: &mut Pen<'_>,
    frame: &mut Frame<'_>,
    s: &State,
    body: Area,
    footer: Area,
) -> AureaResult<()> {
    let lang = s.lang;
    let name = s.info.name.as_str();
    badge(pen, body.x + 22.0, body.y + 30.0, true)?;
    let x = body.x + 60.0;
    pen.clipped(
        &lang.fill(Line::DoneTitle, &[("name", name)]),
        Area::new(x, body.y + 8.0, body.w - 60.0, 24.0),
        &pen.bold(18.0),
        color::TEXT,
    )?;
    let outro = s
        .info
        .brand
        .finish
        .clone()
        .unwrap_or_else(|| lang.fill(Line::DoneBody, &[("name", name)]));
    let used = pen.wrapped(
        &outro,
        Area::new(x, body.y + 38.0, body.w - 60.0, 66.0),
        &pen.font(14.0),
        color::DIM,
    )?;
    if let Some((label, _)) = &s.info.brand.link {
        let font = pen.bold(14.0);
        let link = Area::new(x, body.y + 46.0 + used, pen.width(label, &font), 22.0);
        let hover = frame.hover(Hit::Link);
        let tone = color::mix(pen.accent, color::TEXT, 0.35 + hover * 0.4);
        pen.clipped(label, link, &font, tone)?;
        pen.line(
            (link.x, link.bottom() - 2.0),
            (link.right(), link.bottom() - 2.0),
            1.0,
            color::mix(color::BACKGROUND, tone, 0.3 + hover * 0.7),
        )?;
        frame.hits.push((link, Hit::Link));
    }
    if s.session.wizard.launch.is_some() {
        buttons(
            pen,
            frame,
            footer,
            (&lang.fill(Line::Open, &[("name", name)]), true),
            Some(lang.line(Line::Close)),
        )
    } else {
        buttons(pen, frame, footer, (lang.line(Line::Finish), true), None)
    }
}

fn failed(
    pen: &mut Pen<'_>,
    frame: &mut Frame<'_>,
    s: &State,
    body: Area,
    footer: Area,
) -> AureaResult<()> {
    let lang = s.lang;
    badge(pen, body.x + 22.0, body.y + 22.0, false)?;
    let x = body.x + 60.0;
    pen.text(
        lang.line(Line::FailedTitle),
        x,
        body.y,
        &pen.bold(18.0),
        color::TEXT,
    )?;
    pen.text(
        lang.line(Line::FailedBody),
        x,
        body.y + 28.0,
        &pen.font(14.0),
        color::DIM,
    )?;
    let well = Area::new(body.x, body.y + 60.0, body.w, body.h - 60.0);
    pen.round(well, 10.0, color::SURFACE)?;
    pen.outline(well, 10.0, 1.0, color::RAISED)?;
    let error = s.session.wizard.error.as_deref().unwrap_or_default();
    pen.wrapped(error, well.inset(14.0), &pen.font(13.0), color::BAD)?;
    buttons(pen, frame, footer, (lang.line(Line::Close), true), None)
}
