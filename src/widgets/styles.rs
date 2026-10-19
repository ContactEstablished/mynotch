//! The timer's styles: how its time, its ring and its progress bar are drawn, and the cue
//! that it's running. Picked from a gallery on the Time tab (`gallery.rs`). The names follow
//! BuildNotch's timer themes; the looks are our own.

use std::f32::consts::{PI, TAU};

use super::running_edge;
use crate::gfx::{Align, Canvas, Font, Ink, Path, Rect, Rgb};
use crate::motion;
use crate::theme::{color, space, styles as c, text as type_size};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    /// Rajdhani on a glowing cyan-to-violet arc: the notch's own look.
    #[default]
    Halo,
    /// Chunky pink pixels with a violet shadow, and a ring of blocks.
    Pixel,
    /// Cream on a sunset arc, with a low sun at its end.
    Dusk,
    /// An LED dot matrix, and a ring of dots.
    Dots,
    /// Flip-clock tiles that flip as the seconds go, and an amber kitchen-timer dial.
    Retro,
    /// Green monospace with a blinking cursor, a dashed ring and an ASCII bar.
    Terminal,
    /// Quiet hairlines; the card breathes slowly instead of a light running round it.
    Zen,
    /// Slanted seven-segment digits in brackets, and a ring of ticks with a scanner.
    Hud,
}

impl Style {
    /// In the gallery's order.
    pub const ALL: [Style; 8] =
        [Style::Halo, Style::Pixel, Style::Dusk, Style::Dots, Style::Retro, Style::Terminal, Style::Zen, Style::Hud];

    pub fn name(self) -> &'static str {
        match self {
            Style::Halo => "Halo",
            Style::Pixel => "Pixel",
            Style::Dusk => "Dusk",
            Style::Dots => "Dots",
            Style::Retro => "Retro Clock",
            Style::Terminal => "Terminal",
            Style::Zen => "Zen",
            Style::Hud => "Cyber HUD",
        }
    }

    /// How it's saved in the settings file.
    pub fn key(self) -> &'static str {
        match self {
            Style::Halo => "halo",
            Style::Pixel => "pixel",
            Style::Dusk => "dusk",
            Style::Dots => "dots",
            Style::Retro => "retro",
            Style::Terminal => "terminal",
            Style::Zen => "zen",
            Style::Hud => "hud",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }

    /// Its main colour: the ring, the card's lit edge, the light round a running card.
    pub fn accent(self) -> Rgb {
        match self {
            Style::Halo => color::CYAN,
            Style::Pixel => c::PIXEL,
            Style::Dusk => c::DUSK[1],
            Style::Dots => c::DOTS,
            Style::Retro => c::RETRO,
            Style::Terminal => c::TERMINAL,
            Style::Zen => c::ZEN,
            Style::Hud => c::HUD,
        }
    }

    /// The time, its digits standing on a baseline `baseline` px from the top, from `x`.
    pub fn time(self, cv: &Canvas, face: &Face, x: f32, baseline: f32, size: Size) {
        let plain = |ink| {
            cv.numerals(&[(face.time, size.font(), ink)], x, baseline, Align::Left);
        };
        match self {
            Style::Halo => plain(Ink::Text),
            Style::Dusk => plain(Ink::Rgb(c::DUSK_TEXT)),
            Style::Zen => plain(Ink::Rgb(c::ZEN_TEXT)),
            Style::Pixel => pixels(cv, face.time, x, baseline, size.digit()),
            Style::Dots => dots(cv, face.time, x, baseline, size.digit()),
            Style::Retro => tiles(cv, face, x, baseline, size),
            Style::Terminal => terminal(cv, face, x, baseline, size),
            Style::Hud => segments(cv, face.time, x, baseline, size.digit()),
        }
    }

    /// The status line ("Ends at 4:05 PM") in `r`.
    pub fn status(self, cv: &Canvas, text: &str, r: Rect) {
        match self {
            Style::Terminal => {
                let line = format!("# {}", text.to_lowercase());
                cv.text(&line, Font::MonoSmall, Ink::Rgb(c::TERMINAL_DIM), r, Align::Left);
            }
            Style::Hud => cv.caps(text, Font::Caption, Ink::Rgb(c::HUD_DIM), r, Align::Left),
            Style::Dusk => cv.text(text, Font::Small, Ink::Rgb(c::DUSK_DIM), r, Align::Left),
            _ => cv.text(text, Font::Small, Ink::Dim, r, Align::Left),
        }
    }

    /// The time left as a ring around the start/pause button.
    pub fn ring(self, cv: &Canvas, d: &Dial) {
        match self {
            Style::Halo => halo_ring(cv, d),
            Style::Pixel => pixel_ring(cv, d),
            Style::Dusk => dusk_ring(cv, d),
            Style::Dots => dots_ring(cv, d),
            Style::Retro => retro_ring(cv, d),
            Style::Terminal => terminal_ring(cv, d),
            Style::Zen => zen_ring(cv, d),
            Style::Hud => hud_ring(cv, d),
        }
    }

    /// The time left as a bar along `r` (a couple of px tall; some styles draw a little
    /// above and below it), for the timer card on Home.
    pub fn bar(self, cv: &Canvas, r: Rect, left: f32, running: bool) {
        let left = left.clamp(0.0, 1.0);
        let end = r.x + r.w * left;
        let cy = r.y + r.h / 2.0;
        match self {
            Style::Halo => {
                cv.fill(r, 1.0, Ink::Line);
                cv.fill(Rect { w: (r.w * left).max(r.h), ..r }, 1.0, Ink::Accent);
                if running {
                    let swell = breath(BREATH);
                    cv.halo(end, cy, 2.0 + swell, Ink::Accent, 0.35 + 0.65 * swell);
                }
            }
            Style::Dusk => {
                cv.fill(r, 1.0, Ink::Line);
                cv.fill_gradient(Rect { w: (r.w * left).max(r.h), ..r }, 1.0, &c::DUSK, (r.x, r.right()), 1.0);
                let swell = if running { breath(BREATH) } else { 0.3 };
                cv.halo(end, cy, 2.5, Ink::Rgb(c::DUSK[0]), 0.4 + 0.6 * swell);
                cv.circle(end, cy, 2.5, Ink::Rgb(c::DUSK[0]), None);
            }
            Style::Pixel => {
                let n = ((r.w + 2.0) / 8.0).floor().max(1.0) as usize;
                let pitch = (r.w + 2.0) / n as f32;
                let lit = lit_count(left, n);
                for i in 0..n {
                    let block = Rect::new((r.x + i as f32 * pitch).round(), (cy - 2.0).round(), (pitch - 2.0).round(), 4.0);
                    pixel_block(cv, block, i < lit, running && i + 1 == lit);
                }
            }
            Style::Dots => {
                let n = (r.w / 5.0).floor() as usize + 1;
                let pitch = r.w / (n - 1).max(1) as f32;
                let lit = lit_count(left, n);
                for i in 0..n {
                    let x = r.x + i as f32 * pitch;
                    if i < lit {
                        cv.circle(x, cy, 1.2, Ink::Rgb(c::DOTS), None);
                    } else {
                        cv.circle_a(x, cy, 1.2, Ink::Edge, None, 0.6);
                    }
                }
                if running && lit > 0 {
                    let x = r.x + (lit - 1) as f32 * pitch;
                    let swell = breath(BREATH);
                    cv.halo(x, cy, 1.8, Ink::Rgb(c::DOTS), 0.35 + 0.65 * swell);
                    cv.circle(x, cy, 2.0, Ink::Rgb(c::DOTS_TEXT), None);
                }
            }
            Style::Retro => {
                cv.fill(r, 1.0, Ink::Line);
                for k in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    let x = r.x + r.w * k;
                    cv.stroke_line((x, r.y - 3.0), (x, r.y - 1.5), Ink::Dim, 1.0, 0.6);
                }
                cv.fill(Rect { w: (r.w * left).max(r.h), ..r }, 1.0, Ink::Rgb(c::RETRO));
                cv.circle(end, cy, 2.6, Ink::Rgb(c::RETRO), None);
            }
            Style::Terminal => ascii_bar(cv, r, left),
            Style::Zen => {
                cv.line((r.x, cy), (r.right(), cy), Ink::Line);
                cv.stroke_line((r.x, cy), (end, cy), Ink::Rgb(c::ZEN), 1.0, 0.9);
                let swell = if running { breath(ZEN_BREATH) } else { 0.4 };
                cv.circle(end, cy, 1.4 + 0.8 * swell, Ink::Rgb(c::ZEN), None);
            }
            Style::Hud => {
                let n = (r.w / 4.0).floor() as usize + 1;
                let pitch = r.w / (n - 1).max(1) as f32;
                let lit = lit_count(left, n);
                let (mut on, mut off) = (Path::new(), Path::new());
                for i in 0..n {
                    let x = r.x + i as f32 * pitch;
                    let tick = |p: Path| p.move_to(x, cy - 3.0).line_to(x, cy + 3.0);
                    if i < lit { on = tick(on) } else { off = tick(off) }
                }
                stroke(cv, &off, Ink::Line, 1.0, 1.0);
                stroke(cv, &on, Ink::Rgb(c::HUD), 1.0, 1.0);
                if lit > 0 {
                    let x = r.x + (lit - 1) as f32 * pitch;
                    cv.stroke_line((x, cy - 5.0), (x, cy + 5.0), Ink::Rgb(c::HUD_MARK), 1.5, 1.0);
                }
            }
        }
    }

    /// The cue that the timer is running, on its card `card`: a light running round the
    /// edge, stepping in Pixel; in Zen, the edge breathing slowly instead. (With animations
    /// off, the card's lit edge says it.)
    pub fn edge(self, cv: &Canvas, card: Rect) {
        if motion::reduced() {
            return;
        }
        match self {
            Style::Zen => {
                let k = motion::breath(ZEN_BREATH);
                cv.outline(card, space::CARD_RADIUS, Ink::Rgb(c::ZEN), 0.15 + 0.55 * k);
                cv.outline(card.inset(-2.0), space::CARD_RADIUS + 2.0, Ink::Rgb(c::ZEN), 0.12 * k);
            }
            Style::Pixel => {
                // Hops round in sixteen steps, like a sprite.
                let head = (motion::cycle(1.6) * 16.0).floor() / 16.0;
                cv.runner(card, space::CARD_RADIUS, head, Ink::Rgb(c::PIXEL), 1.0, 0.8);
            }
            _ => running_edge(cv, card, Ink::Rgb(self.accent())),
        }
    }
}

/// How long the glow at the ring's moving end takes to swell and fade.
const BREATH: f32 = 2.4;
/// Zen's slow breath: four seconds in, four out.
const ZEN_BREATH: f32 = 8.0;

/// `motion::breath`, held still with animations off.
fn breath(period: f32) -> f32 {
    if motion::reduced() { 0.6 } else { motion::breath(period) }
}

/// How many of `n` marks are still lit with `left` of the time to go: the last one goes out
/// only at zero.
fn lit_count(left: f32, n: usize) -> usize {
    ((left.clamp(0.0, 1.0) * n as f32).ceil() as usize).min(n)
}

fn stroke(cv: &Canvas, path: &Path, ink: Ink, width: f32, a: f32) {
    if let Some(shape) = cv.path_shape(path) {
        cv.stroke_shape(&shape, ink, width, a);
    }
}

/// What the time shows.
pub struct Face<'a> {
    /// "24:13", or "1:02:03" past an hour.
    pub time: &'a str,
    /// The time shown a second before, and how far (0..1) its digits have flipped over to
    /// `time`. Only Retro Clock flips.
    pub flip: Option<(&'a str, f32)>,
    /// Running (not paused or stopped): cursors blink.
    pub running: bool,
}

/// Where the time is drawn: on a timer card, or small in the gallery.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    Card,
    Tile,
}

impl Size {
    /// How tall the drawn digits are, in px (about the cap height of `font`).
    fn digit(self) -> f32 {
        match self {
            Size::Card => 22.0,
            Size::Tile => 14.0,
        }
    }

    fn font(self) -> Font {
        match self {
            Size::Card => Font::Timer,
            Size::Tile => Font::Fraction,
        }
    }

    /// The monospaced font, and its size in px.
    fn mono(self) -> (Font, f32) {
        match self {
            Size::Card => (Font::MonoTimer, type_size::MONO),
            Size::Tile => (Font::MonoTile, type_size::MONO_TILE),
        }
    }
}

/// A ring around the start/pause button, centred on (`cx`, `cy`).
pub struct Dial {
    pub cx: f32,
    pub cy: f32,
    pub r: f32,
    /// The stroke's width; the styles size their marks from it.
    pub width: f32,
    /// The fraction of the time still to go.
    pub left: f32,
    pub running: bool,
    /// How hovered the button is, 0..=1.
    pub hot: f32,
    /// 1, or less while paused.
    pub on: f32,
}

impl Dial {
    /// The point `turn` of the way round clockwise from 12 o'clock, `r` from the centre.
    fn at(&self, turn: f32, r: f32) -> (f32, f32) {
        let (sin, cos) = (turn * TAU).sin_cos();
        (self.cx + r * sin, self.cy - r * cos)
    }

    /// The ring's moving end.
    fn head(&self) -> (f32, f32) {
        self.at(self.left, self.r)
    }

    /// Ring marks are smaller in the gallery.
    fn small(&self) -> bool {
        self.r < 20.0
    }
}

fn halo_ring(cv: &Canvas, d: &Dial) {
    cv.circle(d.cx, d.cy, d.r, Ink::Line, Some(d.width));
    cv.halo(d.cx, d.cy, d.r, Ink::Accent, (0.55 + 0.45 * d.hot) * d.on);
    cv.ring(d.cx, d.cy, d.r, d.width, d.left, None, d.on);
    if d.running {
        let (hx, hy) = d.head();
        let (swell, k) = (breath(BREATH), d.width / 2.5);
        cv.halo(hx, hy, (2.5 + 1.5 * swell) * k, Ink::Accent, 0.35 + 0.65 * swell);
        cv.circle(hx, hy, 2.4 * k, Ink::Text, None);
    }
}

fn dusk_ring(cv: &Canvas, d: &Dial) {
    cv.circle(d.cx, d.cy, d.r, Ink::Line, Some(d.width));
    cv.halo(d.cx, d.cy, d.r, Ink::Rgb(c::DUSK[1]), (0.4 + 0.4 * d.hot) * d.on);
    cv.ring_gradient(d.cx, d.cy, d.r, d.width, d.left, &c::DUSK, d.on);
    // A low sun at the end of the arc.
    if d.left > 0.0 {
        let (hx, hy) = d.head();
        let (swell, k) = (if d.running { breath(BREATH) } else { 0.3 }, d.width / 2.5);
        cv.halo(hx, hy, 3.2 * k, Ink::Rgb(c::DUSK[0]), (0.45 + 0.55 * swell) * d.on);
        cv.circle_a(hx, hy, 3.2 * k, Ink::Rgb(c::DUSK[0]), None, d.on);
    }
}

/// A square block of a Pixel ring or bar: pink with a violet drop shadow while lit, the end
/// one blinking yellow while it runs.
fn pixel_block(cv: &Canvas, r: Rect, lit: bool, blinking: bool) {
    if !lit {
        cv.fill_a(r, 0.0, Ink::Edge, 0.5);
        return;
    }
    cv.fill(Rect { x: r.x + 1.0, y: r.y + 1.0, ..r }, 0.0, Ink::Rgb(c::PIXEL_SHADOW));
    let on = blinking && !motion::reduced() && motion::cycle(1.0) < 0.5;
    cv.fill(r, 0.0, Ink::Rgb(if on { c::PIXEL_BLINK } else { c::PIXEL }));
}

fn pixel_ring(cv: &Canvas, d: &Dial) {
    let n = if d.small() { 12 } else { 16 };
    let size = (d.width * 2.0).round();
    let lit = lit_count(d.left, n);
    cv.halo(d.cx, d.cy, d.r, Ink::Rgb(c::PIXEL), 0.4 * d.hot * d.on);
    for i in 0..n {
        let (x, y) = d.at((i as f32 + 0.5) / n as f32, d.r);
        let block = Rect::new((x - size / 2.0).round(), (y - size / 2.0).round(), size, size);
        pixel_block(cv, block, i < lit, d.running && i + 1 == lit);
    }
}

fn dots_ring(cv: &Canvas, d: &Dial) {
    let n = if d.small() { 24 } else { 40 };
    let dot = d.width * 0.55;
    let lit = lit_count(d.left, n);
    cv.halo(d.cx, d.cy, d.r, Ink::Rgb(c::DOTS), 0.45 * d.hot * d.on);
    for i in 0..n {
        let (x, y) = d.at((i as f32 + 0.5) / n as f32, d.r);
        if i < lit {
            cv.circle_a(x, y, dot, Ink::Rgb(c::DOTS), None, d.on);
        } else {
            cv.circle_a(x, y, dot, Ink::Edge, None, 0.6);
        }
    }
    if d.running && lit > 0 {
        let (x, y) = d.at((lit as f32 - 0.5) / n as f32, d.r);
        let swell = breath(BREATH);
        cv.halo(x, y, dot * 1.6, Ink::Rgb(c::DOTS), 0.35 + 0.65 * swell);
        cv.circle(x, y, dot * 1.5, Ink::Rgb(c::DOTS_TEXT), None);
    }
}

fn retro_ring(cv: &Canvas, d: &Dial) {
    cv.circle(d.cx, d.cy, d.r, Ink::Line, Some(d.width));
    // Twelve marks round the outside, like a kitchen timer's dial; every third a bit bolder.
    let r0 = d.r + d.width / 2.0 + 1.5;
    let r1 = r0 + d.r * 0.1;
    for i in 0..12 {
        let t = i as f32 / 12.0;
        let (width, a) = if i % 3 == 0 { (1.4, 0.75) } else { (1.0, 0.4) };
        cv.stroke_line(d.at(t, r0), d.at(t, r1), Ink::Dim, width, a);
    }
    cv.halo(d.cx, d.cy, d.r, Ink::Rgb(c::RETRO), (0.3 + 0.4 * d.hot) * d.on);
    cv.ring(d.cx, d.cy, d.r, d.width, d.left, Some(Ink::Rgb(c::RETRO)), d.on);
    if d.left > 0.0 {
        let (hx, hy) = d.head();
        cv.circle_a(hx, hy, d.width * 1.15, Ink::Rgb(c::RETRO), None, d.on);
        cv.circle_a(hx, hy, d.width * 0.45, Ink::Rgb(c::RETRO_TILE), None, d.on);
    }
}

fn terminal_ring(cv: &Canvas, d: &Dial) {
    let n = if d.small() { 16 } else { 24 };
    let lit = lit_count(d.left, n);
    // Dashes: each a short arc, with a gap either side.
    let gap = 0.22 / n as f32;
    let (mut on, mut off) = (Path::new(), Path::new());
    for i in 0..n {
        let (a, b) = (d.at(i as f32 / n as f32 + gap, d.r), d.at((i + 1) as f32 / n as f32 - gap, d.r));
        let dash = |p: Path| p.move_to(a.0, a.1).arc_to(b.0, b.1, (d.r, d.r), false, true);
        if i < lit { on = dash(on) } else { off = dash(off) }
    }
    cv.halo(d.cx, d.cy, d.r, Ink::Rgb(c::TERMINAL), 0.4 * d.hot * d.on);
    stroke(cv, &off, Ink::Rgb(c::TERMINAL), d.width, 0.15);
    stroke(cv, &on, Ink::Rgb(c::TERMINAL), d.width, d.on);
}

fn zen_ring(cv: &Canvas, d: &Dial) {
    cv.circle_a(d.cx, d.cy, d.r, Ink::Line, Some(1.0), 0.8);
    cv.halo(d.cx, d.cy, d.r, Ink::Rgb(c::ZEN), 0.3 * d.hot * d.on);
    cv.ring(d.cx, d.cy, d.r, d.width * 0.6, d.left, Some(Ink::Rgb(c::ZEN)), 0.9 * d.on);
    if d.left > 0.0 {
        let (hx, hy) = d.head();
        let swell = if d.running { breath(ZEN_BREATH) } else { 0.4 };
        cv.circle_a(hx, hy, d.width * (0.6 + 0.5 * swell), Ink::Rgb(c::ZEN), None, d.on);
    }
}

fn hud_ring(cv: &Canvas, d: &Dial) {
    let n = if d.small() { 32 } else { 48 };
    let lit = lit_count(d.left, n);
    let (r0, r1) = (d.r - d.width, d.r + d.width * 0.6);
    let (mut on, mut off) = (Path::new(), Path::new());
    for i in 0..n {
        let t = i as f32 / n as f32;
        let ((x0, y0), (x1, y1)) = (d.at(t, r0), d.at(t, r1));
        let tick = |p: Path| p.move_to(x0, y0).line_to(x1, y1);
        if i < lit { on = tick(on) } else { off = tick(off) }
    }
    cv.halo(d.cx, d.cy, d.r, Ink::Rgb(c::HUD), 0.45 * d.hot * d.on);
    stroke(cv, &off, Ink::Line, 1.0, 1.0);
    stroke(cv, &on, Ink::Rgb(c::HUD), 1.0, d.on);
    // The end, marked in magenta.
    if d.left > 0.0 {
        let (a, b) = (d.at(d.left, r0 - 2.0), d.at(d.left, r1 + 2.0));
        cv.stroke_line(a, b, Ink::Rgb(c::HUD_MARK), 1.6, d.on);
    }
    // A scanner sweeping round outside while it runs.
    if d.running && !motion::reduced() {
        let t = motion::cycle(3.0);
        let r = r1 + 3.0;
        let ((x0, y0), (x1, y1)) = (d.at(t, r), d.at(t + 0.12, r));
        stroke(cv, &Path::new().move_to(x0, y0).arc_to(x1, y1, (r, r), false, true), Ink::Rgb(c::HUD_MARK), 1.0, 0.6);
    }
}

/// Terminal's bar: `[#######---------]` in small monospace.
fn ascii_bar(cv: &Canvas, r: Rect, left: f32) {
    let cell = cv.measure("#", Font::MonoSmall).max(1.0);
    let n = ((r.w / cell).floor() as usize).saturating_sub(2).max(1);
    let lit = lit_count(left, n);
    let line = Rect::new(r.x, r.y + r.h / 2.0 - 8.0, r.w, 16.0);
    let (green, dim) = (Ink::Rgb(c::TERMINAL), Ink::Rgb(c::TERMINAL_DIM));
    cv.text(&format!("[{}", "#".repeat(lit)), Font::MonoSmall, green, line, Align::Left);
    let rest = Rect { x: line.x + (lit + 1) as f32 * cell, w: line.w - (lit + 1) as f32 * cell + 4.0, ..line };
    cv.text(&format!("{}]", "-".repeat(n - lit)), Font::MonoSmall, dim, rest, Align::Left);
}

/// 3x5 pixel digits, a row per byte with the leftmost pixel in bit 2; then the colon, one
/// pixel wide.
const PIXEL_FONT: [[u8; 5]; 11] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b011, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
    [0b0, 0b1, 0b0, 0b1, 0b0],
];

/// 5x7 dot-matrix digits, the leftmost dot in bit 4; then the colon, one dot wide.
const DOT_FONT: [[u8; 7]; 11] = [
    [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
    [0b01110, 0b10001, 0b00001, 0b00110, 0b00001, 0b10001, 0b01110],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
    [0b0, 0b0, 0b1, 0b0, 0b1, 0b0, 0b0],
];

/// Lays `s` out in a bitmap font whose digits are `cols` cells wide, a cell apart: every
/// cell as (column, row, lit), and the width in cells. Anything but digits and colons is
/// skipped.
fn bitmap<const ROWS: usize>(s: &str, font: &[[u8; ROWS]; 11], cols: usize) -> (Vec<(usize, usize, bool)>, usize) {
    let mut cells = Vec::new();
    let mut at = 0;
    for ch in s.chars() {
        let (glyph, w) = match ch {
            ':' => (10, 1),
            _ => match ch.to_digit(10) {
                Some(d) => (d as usize, cols),
                None => continue,
            },
        };
        for (row, bits) in font[glyph].iter().enumerate() {
            for col in 0..w {
                cells.push((at + col, row, bits >> (w - 1 - col) & 1 == 1));
            }
        }
        at += w + 1;
    }
    (cells, at.saturating_sub(1))
}

/// A square of side `p` at (`x`, `y`), as a figure of `path`.
fn square(path: Path, x: f32, y: f32, p: f32) -> Path {
    path.move_to(x, y).line_to(x + p, y).line_to(x + p, y + p).line_to(x, y + p).close()
}

/// A circle as a figure of `path`.
fn disc(path: Path, cx: f32, cy: f32, r: f32) -> Path {
    path.move_to(cx - r, cy).arc_to(cx + r, cy, (r, r), false, true).arc_to(cx - r, cy, (r, r), false, true).close()
}

/// Pixel's digits: whole-px blocks, so they stay crisp, over a violet drop shadow.
fn pixels(cv: &Canvas, s: &str, x: f32, baseline: f32, digit: f32) {
    let p = (digit / 5.0).round();
    let (x, top) = (x.round(), baseline - p * 5.0);
    let (cells, _) = bitmap(s, &PIXEL_FONT, 3);
    let lit = || cells.iter().filter(|c| c.2).map(|&(col, row, _)| (x + col as f32 * p, top + row as f32 * p));
    let drop = (p / 3.0).round().max(1.0);
    let shadow = lit().fold(Path::new(), |path, (cx, cy)| square(path, cx + drop, cy + drop, p));
    let face = lit().fold(Path::new(), |path, (cx, cy)| square(path, cx, cy, p));
    for (path, ink) in [(shadow, c::PIXEL_SHADOW), (face, c::PIXEL)] {
        if let Some(shape) = cv.path_shape(&path) {
            cv.fill_shape(&shape, Ink::Rgb(ink), 1.0);
        }
    }
}

/// Dots' digits: an LED matrix, the unlit dots faintly there.
fn dots(cv: &Canvas, s: &str, x: f32, baseline: f32, digit: f32) {
    let p = digit / 7.0;
    let (r, top) = (p * 0.36, baseline - digit);
    let (mut lit, mut glow, mut unlit) = (Path::new(), Path::new(), Path::new());
    for (col, row, on) in bitmap(s, &DOT_FONT, 5).0 {
        let (cx, cy) = (x + (col as f32 + 0.5) * p, top + (row as f32 + 0.5) * p);
        if on {
            lit = disc(lit, cx, cy, r);
            glow = disc(glow, cx, cy, r * 1.7);
        } else {
            unlit = disc(unlit, cx, cy, r);
        }
    }
    // Fills only: stroking this many circles every frame costs several times as much.
    for (path, ink, a) in [(unlit, Ink::Edge, 0.35), (glow, Ink::Rgb(c::DOTS), 0.22), (lit, Ink::Rgb(c::DOTS_TEXT), 1.0)] {
        if let Some(shape) = cv.path_shape(&path) {
            cv.fill_shape(&shape, ink, a);
        }
    }
}

/// Retro Clock's digits: each on a flip tile. A digit that just changed flips over: the old
/// top half falls on its hinge, showing the new top behind it, and the new bottom half lands
/// over the old.
fn tiles(cv: &Canvas, face: &Face, x: f32, baseline: f32, size: Size) {
    let (d, font) = (size.digit(), size.font());
    let pad = (d * 0.2).round();
    let tile_w = (d * 0.95).round();
    let top = baseline - d - pad;
    let hinge = top + (d + pad * 2.0) / 2.0;
    // Only when the two times line up digit for digit.
    let before = face.flip.filter(|(prev, _)| prev.len() == face.time.len());
    let mut at = x;
    for (i, ch) in face.time.chars().enumerate() {
        if ch == ':' {
            for dy in [-0.2, 0.2] {
                cv.circle_a(at + d * 0.15, hinge + dy * d, d * 0.07, Ink::Rgb(c::RETRO_TEXT), None, 0.8);
            }
            at += d * 0.3;
            continue;
        }
        let tile = Rect::new(at, top, tile_w, d + pad * 2.0);
        let was = before.and_then(|(prev, t)| prev.chars().nth(i).filter(|&p| p != ch).map(|p| (p, t)));
        match was {
            None => flap(cv, tile, ch, font, baseline, None),
            Some((was, t)) => {
                flap(cv, tile, ch, font, baseline, Some(Half::Top));
                flap(cv, tile, was, font, baseline, Some(Half::Bottom));
                // cos runs 1 → -1: the old top folds down to edge-on, then the new bottom
                // folds down from edge-on.
                let turn = (PI * t.clamp(0.0, 1.0)).cos();
                let (glyph, half, k) = if turn > 0.0 { (was, Half::Top, turn) } else { (ch, Half::Bottom, -turn) };
                cv.squashed(hinge, k, || {
                    flap(cv, tile, glyph, font, baseline, Some(half));
                    // Darker as it turns away from the light.
                    let shade = Rect { y: if half == Half::Top { tile.y } else { hinge }, h: tile.h / 2.0, ..tile };
                    cv.fill_a(shade, 0.0, Ink::Rgb(color::SHADOW), 0.5 * (1.0 - k));
                });
            }
        }
        // The split between the halves.
        cv.line((tile.x, hinge), (tile.right(), hinge), Ink::Rgb(color::SHADOW));
        at += tile_w + (d * 0.12).round();
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Half {
    Top,
    Bottom,
}

/// A flip tile showing `ch`, whole or just one half.
fn flap(cv: &Canvas, tile: Rect, ch: char, font: Font, baseline: f32, half: Option<Half>) {
    let mid = tile.y + tile.h / 2.0;
    let top = Rect { h: tile.h / 2.0, ..tile };
    let clip = match half {
        None => tile,
        Some(Half::Top) => top,
        Some(Half::Bottom) => Rect { y: mid, ..top },
    };
    cv.push_clip(clip);
    cv.fill_a(tile, 3.0, Ink::Rgb(c::RETRO_TILE), 0.92);
    // Lighter above the split, as if lit from above.
    cv.push_clip(top);
    cv.fill(tile, 3.0, Ink::Track);
    cv.pop_clip();
    let mut buf = [0; 4];
    cv.numerals(&[(ch.encode_utf8(&mut buf), font, Ink::Rgb(c::RETRO_TEXT))], tile.x + tile.w / 2.0, baseline, Align::Center);
    cv.pop_clip();
}

/// Terminal's digits: phosphor-green monospace, faintly glowing, with a block cursor that
/// blinks while it runs.
fn terminal(cv: &Canvas, face: &Face, x: f32, baseline: f32, size: Size) {
    let (font, px) = size.mono();
    let green = Ink::Rgb(c::TERMINAL);
    // Whole lines rather than `numerals`' digit by digit: the same in a monospaced font, and
    // a fraction of the cost, which counts here since this redraws every frame.
    cv.faded(green, 0.25, || {
        for dx in [-0.9, 0.9] {
            cv.text_at(face.time, font, green, x + dx, baseline);
        }
    });
    cv.text_at(face.time, font, green, x, baseline);
    let w = cv.measure(face.time, font);
    let cell = cv.measure("0", font);
    let cursor = Rect::new(x + w + cell * 0.2, baseline - px * 0.72, cell * 0.8, px * 0.84);
    let on = !face.running || motion::reduced() || motion::cycle(1.0) < 0.55;
    if on {
        cv.fill_a(cursor, 0.0, green, if face.running { 0.9 } else { 0.5 });
    }
}

/// Seven-segment digits: segments a to g in bits 0 to 6.
const SEGMENTS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];
/// How far the segments lean: x moves this many px per px up.
const LEAN: f32 = 0.12;

/// Cyber HUD's digits: slanted seven-segment, the unlit segments faintly there, in
/// corner brackets.
fn segments(cv: &Canvas, s: &str, x: f32, baseline: f32, d: f32) {
    let (w, t) = (d * 0.56, d * 0.14);
    let (h, gap) = (t / 2.0, t * 0.2);
    let (top, mid) = (baseline - d, baseline - d / 2.0);
    let poly = |path: Path, points: &[(f32, f32)]| {
        let lean = |(px, py): (f32, f32)| (px + (baseline - py) * LEAN, py);
        let (x0, y0) = lean(points[0]);
        points[1..].iter().fold(path.move_to(x0, y0), |p, &pt| {
            let (px, py) = lean(pt);
            p.line_to(px, py)
        })
        .close()
    };
    let across = |x0: f32, x1: f32, y: f32| [(x0, y), (x0 + h, y - h), (x1 - h, y - h), (x1, y), (x1 - h, y + h), (x0 + h, y + h)];
    let down = |x: f32, y0: f32, y1: f32| [(x, y0), (x + h, y0 + h), (x + h, y1 - h), (x, y1), (x - h, y1 - h), (x - h, y0 + h)];
    let (mut lit, mut unlit) = (Path::new(), Path::new());
    let mut at = x;
    for ch in s.chars() {
        if ch == ':' {
            for cy in [top + d * 0.3, top + d * 0.72] {
                lit = poly(lit, &[(at, cy - h), (at + t, cy - h), (at + t, cy + h), (at, cy + h)]);
            }
            at += t + d * 0.22;
            continue;
        }
        let Some(mask) = ch.to_digit(10).map(|n| SEGMENTS[n as usize]) else { continue };
        let (l, r) = (at + h, at + w - h);
        let segs = [
            across(l + gap, r - gap, top + h),
            down(r, top + h + gap, mid - gap),
            down(r, mid + gap, baseline - h - gap),
            across(l + gap, r - gap, baseline - h),
            down(l, mid + gap, baseline - h - gap),
            down(l, top + h + gap, mid - gap),
            across(l + gap, r - gap, mid),
        ];
        for (k, seg) in segs.iter().enumerate() {
            if mask >> k & 1 == 1 { lit = poly(lit, seg) } else { unlit = poly(unlit, seg) }
        }
        at += w + d * 0.22;
    }
    let ink = Ink::Rgb(c::HUD);
    if let Some(shape) = cv.path_shape(&unlit) {
        cv.fill_shape(&shape, ink, 0.08);
    }
    if let Some(shape) = cv.path_shape(&lit) {
        cv.stroke_shape(&shape, ink, 2.5, 0.16);
        cv.fill_shape(&shape, ink, 1.0);
    }
    // Corner brackets round the readout.
    let (l, r) = (x - 5.0, at - d * 0.22 + d * LEAN + 4.0);
    let (t0, b0) = (top - 4.0, baseline + 4.0);
    let arm = (d * 0.25).round();
    let brackets = Path::new()
        .move_to(l, t0 + arm).line_to(l, t0).line_to(l + arm, t0)
        .move_to(r - arm, t0).line_to(r, t0).line_to(r, t0 + arm)
        .move_to(l, b0 - arm).line_to(l, b0).line_to(l + arm, b0)
        .move_to(r - arm, b0).line_to(r, b0).line_to(r, b0 - arm);
    stroke(cv, &brackets, ink, 1.0, 0.6);
}

#[cfg(test)]
mod tests {
    use super::{DOT_FONT, PIXEL_FONT, SEGMENTS, Style, bitmap, lit_count};

    #[test]
    fn keys_round_trip_and_names_are_unique() {
        for s in Style::ALL {
            assert_eq!(Style::from_key(s.key()), Some(s));
            assert_eq!(Style::ALL.iter().filter(|o| o.name() == s.name()).count(), 1);
        }
        assert_eq!(Style::from_key("plaid"), None);
        assert_eq!(Style::default(), Style::Halo);
    }

    #[test]
    fn bitmap_glyphs_fit_their_widths_and_differ() {
        for (glyphs, cols) in [(PIXEL_FONT.map(|g| g.to_vec()), 3), (DOT_FONT.map(|g| g.to_vec()), 5)] {
            for (i, g) in glyphs.iter().enumerate() {
                let w = if i == 10 { 1 } else { cols };
                assert!(g.iter().all(|&row| row >> w == 0), "glyph {i} is wider than {w}");
                assert!(g.iter().any(|&row| row != 0), "glyph {i} is blank");
            }
            for a in 0..10 {
                for b in a + 1..10 {
                    assert_ne!(glyphs[a], glyphs[b], "{a} and {b} look the same");
                }
            }
        }
        let distinct = SEGMENTS.iter().collect::<std::collections::HashSet<_>>().len();
        assert_eq!(distinct, 10);
    }

    #[test]
    fn bitmap_layout_spaces_glyphs_a_cell_apart() {
        let (cells, width) = bitmap("25:00", &PIXEL_FONT, 3);
        assert_eq!(width, 4 * 3 + 1 + 4, "four digits, a colon, four gaps");
        assert_eq!(cells.len(), 4 * 3 * 5 + 5);
        assert!(cells.iter().all(|&(col, row, _)| col < width && row < 5));
        // The colon's two pixels, in the column after "25" and its gap.
        let colon: Vec<_> = cells.iter().filter(|c| c.0 == 8 && c.2).map(|c| c.1).collect();
        assert_eq!(colon, [1, 3]);
        assert_eq!(bitmap("1:02:03", &DOT_FONT, 5).1, 5 * 5 + 2 + 6);
        assert_eq!(bitmap("", &DOT_FONT, 5).1, 0);
    }

    #[test]
    fn drawn_times_fit_the_timer_card_even_past_an_hour() {
        // Pixel's digits are 4 px a cell on a card, Dots' 22/7; the Time tab's card leaves
        // about 137 px left of the ring (see focus.rs).
        assert!(bitmap("2:30:00", &PIXEL_FONT, 3).1 as f32 * 4.0 < 137.0);
        assert!(bitmap("2:30:00", &DOT_FONT, 5).1 as f32 * 22.0 / 7.0 < 137.0);
    }

    #[test]
    fn the_last_mark_goes_out_only_at_zero() {
        assert_eq!(lit_count(1.0, 16), 16);
        assert_eq!(lit_count(0.999, 16), 16);
        assert_eq!(lit_count(0.01, 16), 1);
        assert_eq!(lit_count(0.0, 16), 0);
        assert_eq!(lit_count(1.5, 16), 16);
    }
}
