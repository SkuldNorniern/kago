//! drawing on the setup canvas, after Lykil and Aira Studio's `draw`:
//! one palette, rounded panels, text that fits, eased values.

use std::collections::HashMap;
use std::time::Instant;

use aurea::AureaResult;
use aurea::render::{
    Color, DrawingContext, Font, FontWeight, GradientStop, LinearGradient, Paint, PaintStyle, Path,
    PathCommand, Point, Rect,
};

use super::super::text::Lang;

pub mod color {
    use aurea::render::Color;

    pub const BACKGROUND: Color = Color::rgb(15, 16, 22);
    pub const SURFACE: Color = Color::rgb(24, 26, 34);
    pub const RAISED: Color = Color::rgb(34, 37, 48);
    pub const HOVER: Color = Color::rgb(46, 50, 64);
    pub const BORDER: Color = Color::rgb(54, 58, 74);
    pub const TEXT: Color = Color::rgb(228, 230, 238);
    pub const DIM: Color = Color::rgb(160, 165, 182);
    pub const FAINT: Color = Color::rgb(118, 123, 142);
    pub const ACCENT: Color = Color::rgb(128, 150, 255);
    pub const ACCENT_TEXT: Color = Color::rgb(12, 16, 36);
    pub const GOOD: Color = Color::rgb(118, 208, 150);
    pub const WARN: Color = Color::rgb(240, 200, 110);
    pub const BAD: Color = Color::rgb(240, 116, 106);

    pub fn mix(c: Color, other: Color, t: f32) -> Color {
        let m = |a: u8, b: u8| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let v =
                (f32::from(a) + (f32::from(b) - f32::from(a)) * t.clamp(0.0, 1.0)).round() as u8;
            v
        };
        Color::rgb(m(c.r, other.r), m(c.g, other.g), m(c.b, other.b))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Area {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn inset(&self, d: f32) -> Self {
        Self::new(
            self.x + d,
            self.y + d,
            (self.w - 2.0 * d).max(0.0),
            (self.h - 2.0 * d).max(0.0),
        )
    }

    pub fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }
}

/// values eased toward a target each frame.
#[derive(Debug, Default)]
pub struct Anim {
    values: HashMap<u32, f32>,
    last: Option<Instant>,
    dt: f32,
    busy: bool,
}

impl Anim {
    pub fn frame(&mut self) {
        let now = Instant::now();
        self.dt = self
            .last
            .map_or(0.0, |l| now.duration_since(l).as_secs_f32().min(1.0 / 30.0));
        self.last = Some(now);
        self.busy = false;
    }

    pub const fn busy(&self) -> bool {
        self.busy
    }

    /// `key` moved toward `target` at `rate` per second; a new key starts
    /// at its target.
    pub fn to(&mut self, key: u32, target: f32, rate: f32) -> f32 {
        let dt = self.dt;
        let now = self.values.entry(key).or_insert(target);
        *now += (target - *now) * (1.0 - (-rate * dt).exp());
        if (target - *now).abs() < 0.002 {
            *now = target;
        } else {
            self.busy = true;
        }
        *now
    }

    pub fn set(&mut self, key: u32, value: f32) {
        self.values.insert(key, value);
        self.busy = true;
    }
}

pub fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub struct Pen<'a> {
    pub ctx: &'a mut dyn DrawingContext,
    pub lang: Lang,
    /// the main color: kago's, or the package's brand accent.
    pub accent: Color,
    /// text and marks drawn on `accent`.
    pub on_accent: Color,
}

/// dark text on a light accent, light text on a dark one.
pub fn on(accent: Color) -> Color {
    let luma =
        0.2126 * f32::from(accent.r) + 0.7152 * f32::from(accent.g) + 0.0722 * f32::from(accent.b);
    if luma > 140.0 {
        color::ACCENT_TEXT
    } else {
        color::TEXT
    }
}

impl Pen<'_> {
    pub fn font(&self, size: f32) -> Font {
        Font::new(self.lang.font_family(), size)
    }

    pub fn bold(&self, size: f32) -> Font {
        self.font(size).with_weight(FontWeight::Bold)
    }

    pub fn fill(&mut self, area: Area, c: Color) -> AureaResult<()> {
        self.ctx.draw_rect(area.rect(), &fill(c))
    }

    pub fn veil(&mut self, area: Area, c: Color, alpha: f32) -> AureaResult<()> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.ctx
            .draw_rect(area.rect(), &fill(Color::rgba(c.r, c.g, c.b, a)))
    }

    /// `c` over `area`, going from `from` to `to` opacity: top to bottom,
    /// or left to right when `across`.
    pub fn fade(
        &mut self,
        area: Area,
        c: Color,
        (from, to): (f32, f32),
        across: bool,
    ) -> AureaResult<()> {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let at =
            |alpha: f32| Color::rgba(c.r, c.g, c.b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8);
        let end = if across {
            Point::new(area.right(), area.y)
        } else {
            Point::new(area.x, area.bottom())
        };
        let gradient = LinearGradient {
            start: Point::new(area.x, area.y),
            end,
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: at(from),
                },
                // reached a little early and held, so the far edge is
                // exactly `to` and nothing next to it shows a step.
                GradientStop {
                    offset: 0.9,
                    color: at(to),
                },
                GradientStop {
                    offset: 1.0,
                    color: at(to),
                },
            ],
        };
        self.ctx.fill_linear_gradient(&gradient, area.rect())
    }

    pub fn round(&mut self, area: Area, radius: f32, c: Color) -> AureaResult<()> {
        self.ctx.draw_path(&rounded(area, radius), &fill(c))
    }

    pub fn outline(&mut self, area: Area, radius: f32, width: f32, c: Color) -> AureaResult<()> {
        self.ctx
            .draw_path(&rounded(area.inset(width / 2.0), radius), &stroke(c, width))
    }

    pub fn line(&mut self, from: (f32, f32), to: (f32, f32), w: f32, c: Color) -> AureaResult<()> {
        self.ctx
            .draw_line(from.0, from.1, to.0, to.1, &stroke(c, w))
    }

    pub fn circle(&mut self, x: f32, y: f32, r: f32, c: Color) -> AureaResult<()> {
        self.ctx.draw_circle(Point::new(x, y), r, &fill(c))
    }

    pub fn width(&mut self, text: &str, font: &Font) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let guess = text.chars().count() as f32 * font.size * 0.55;
        self.ctx.measure_text(text, font).map_or(guess, |m| m.width)
    }

    /// `text` with its top at `y`.
    pub fn text(&mut self, text: &str, x: f32, y: f32, font: &Font, c: Color) -> AureaResult<()> {
        self.ctx
            .draw_text_with_font(text, Point::new(x, y + font.size * 0.8), font, &fill(c))
    }

    pub fn centred(&mut self, text: &str, area: Area, font: &Font, c: Color) -> AureaResult<()> {
        let w = self.width(text, font);
        let x = area.x + (area.w - w) / 2.0;
        let y = area.y + (area.h - font.size) / 2.0;
        self.text(text, x, y, font, c)
    }

    /// one line at the left of `area`, centred up and down, cut with an
    /// ellipsis when too long.
    pub fn clipped(&mut self, text: &str, area: Area, font: &Font, c: Color) -> AureaResult<()> {
        let y = area.y + (area.h - font.size) / 2.0;
        if self.width(text, font) <= area.w {
            return self.text(text, area.x, y, font, c);
        }
        let mut shown = text.to_owned();
        while self.width(&shown, font) > area.w && shown.chars().count() > 1 {
            shown = shown.trim_end_matches('\u{2026}').to_owned();
            shown.pop();
            shown.push('\u{2026}');
        }
        self.text(&shown, area.x, y, font, c)
    }

    /// `text` broken into lines that fit `width`, at spaces when it can.
    /// keeps the text's own line breaks.
    pub fn lines(&mut self, text: &str, width: f32, font: &Font) -> Vec<String> {
        let mut out = Vec::new();
        for paragraph in text.lines() {
            let mut rest = paragraph.trim_end();
            if rest.is_empty() {
                out.push(String::new());
            }
            while !rest.is_empty() {
                let mut end = rest.len();
                if self.width(rest, font) > width {
                    let mut fits = 0;
                    let mut space = None;
                    for (i, ch) in rest.char_indices() {
                        let next = i + ch.len_utf8();
                        if self.width(&rest[..next], font) > width {
                            break;
                        }
                        fits = next;
                        if ch == ' ' {
                            space = Some(i);
                        }
                    }
                    end = space
                        .filter(|s| *s > 0)
                        .unwrap_or(fits.max(rest.chars().next().map_or(1, char::len_utf8)));
                }
                out.push(rest[..end].trim_end().to_owned());
                rest = rest[end..].trim_start();
            }
        }
        out
    }

    /// wrapped `text` from the top of `area`, as many lines as fit.
    /// returns the height used.
    pub fn wrapped(&mut self, text: &str, area: Area, font: &Font, c: Color) -> AureaResult<f32> {
        let line_h = font.size + 6.0;
        let mut y = area.y;
        for line in self.lines(text, area.w, font) {
            if y + line_h > area.bottom() + 0.5 {
                break;
            }
            self.text(&line, area.x, y, font, c)?;
            y += line_h;
        }
        Ok(y - area.y)
    }
}

pub fn fill(c: Color) -> Paint {
    Paint::new().color(c).style(PaintStyle::Fill)
}

pub fn stroke(c: Color, width: f32) -> Paint {
    Paint::new()
        .color(c)
        .style(PaintStyle::Stroke)
        .stroke_width(width)
}

fn rounded(a: Area, radius: f32) -> Path {
    let r = radius.min(a.w / 2.0).min(a.h / 2.0).max(0.0);
    let k = r * 0.552_284_8;
    let (x0, y0, x1, y1) = (a.x, a.y, a.right(), a.bottom());
    let p = Point::new;
    let mut path = Path::new();
    path.commands = vec![
        PathCommand::MoveTo(p(x0 + r, y0)),
        PathCommand::LineTo(p(x1 - r, y0)),
        PathCommand::CubicTo(p(x1 - r + k, y0), p(x1, y0 + r - k), p(x1, y0 + r)),
        PathCommand::LineTo(p(x1, y1 - r)),
        PathCommand::CubicTo(p(x1, y1 - r + k), p(x1 - r + k, y1), p(x1 - r, y1)),
        PathCommand::LineTo(p(x0 + r, y1)),
        PathCommand::CubicTo(p(x0 + r - k, y1), p(x0, y1 - r + k), p(x0, y1 - r)),
        PathCommand::LineTo(p(x0, y0 + r)),
        PathCommand::CubicTo(p(x0, y0 + r - k), p(x0 + r - k, y0), p(x0 + r, y0)),
        PathCommand::Close,
    ];
    path
}
