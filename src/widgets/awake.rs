//! The Awake tab: keep the PC awake (see `crate::awake`). An eye on the left is the switch,
//! closed while the PC may sleep and open, blinking now and then, while it's kept awake. On the
//! right, how long for: chips for the usual lengths and a slider for the rest, in steps of 15
//! minutes up to 8 hours, then no limit.

use std::rc::Rc;
use std::time::{Duration, Instant};

use super::time::{caption, header};
use super::{Compact, Widget, clock, slider, slider_at, slider_hit};
use crate::awake::{Awake, Length};
use crate::gfx::{Align, Canvas, Font, Ink, Path, Rect, glyph};
use crate::motion;
use crate::settings::Settings;
use crate::theme::{color, motion as timing, space};

/// Lengths offered as chips.
const PRESETS: [(Length, &str); 5] = [
    (Length::Minutes(30), "30 m"),
    (Length::Minutes(60), "1 h"),
    (Length::Minutes(120), "2 h"),
    (Length::Minutes(240), "4 h"),
    (Length::Forever, "∞"),
];
/// The eye's card, on the left; the length's card takes the rest.
const EYE_CARD_W: f32 = 220.0;
const EYE_SIZE: f32 = 92.0;
const CHIP_W: f32 = 44.0;
const CHIP_H: f32 = 21.0;
const CHIP_GAP: f32 = 6.0;
/// Room right of the slider for its value.
const VALUE_W: f32 = 64.0;
/// Where the big readout sits (its baseline) in the length's card.
const BASELINE: f32 = 46.0;
/// How long the eye takes to open or close.
const OPEN: Duration = Duration::from_millis(450);
/// The eye blinks once every `BLINK_EVERY` seconds, taking `BLINK` seconds.
const BLINK_EVERY: f32 = 5.0;
const BLINK: f32 = 0.22;

pub struct AwakeWidget {
    awake: Rc<Awake>,
    /// Content size at the last press, for following a drag.
    size: (f32, f32),
}

impl AwakeWidget {
    pub fn new(awake: Rc<Awake>) -> Self {
        Self { awake, size: (692.0, 146.0) }
    }

    fn draw_eye_card(&self, cv: &Canvas, card: Rect, now: Instant) {
        let on = self.awake.on();
        let lit = cv.ease(motion::key("awake-card"), f32::from(u8::from(on)), timing::STATE);
        cv.card(card, space::CARD_RADIUS, lit);
        cv.fill_a(card, space::CARD_RADIUS, Ink::Hover, 0.5 * cv.hover(card));
        let r = card.inset(space::CARD_PAD);
        let c = cv.offset(r.x, r.y);
        header(&c, None, "Awake", if on { Ink::Accent } else { Ink::Dim }, r.w);

        let open = cv.ease(motion::key("awake-eye"), f32::from(u8::from(on)), OPEN);
        let at = (r.w / 2.0, 58.0);
        if open > 0.0 && !motion::reduced() {
            c.halo(at.0, at.1, EYE_SIZE * 0.3, Ink::Accent, open * (0.35 + 0.25 * motion::breath(4.0)));
        }
        eye(&c, at, EYE_SIZE, open * blink(), if on { Ink::Accent } else { Ink::Dim });

        let status = match self.awake.note(now) {
            Some(why) if !on => why.note().to_string(),
            _ if on => "On · tap to let it sleep".into(),
            _ => "Off · tap to stay awake".into(),
        };
        c.text(&status, Font::Small, Ink::Dim, Rect::new(0.0, r.h - 16.0, r.w, 16.0), Align::Center);
    }

    fn draw_length(&self, cv: &Canvas, w: f32, h: f32, now: Instant) {
        let (on, length) = (self.awake.on(), self.awake.length());
        caption(cv, 0.0, if on { "Time left" } else { "Stay awake for" }, Ink::Dim, w);
        let remaining = self.awake.remaining(now);
        // Rounded up to whole seconds, so it reads 1:00:00, not 59:59, at the start.
        let shown = remaining.map(|d| clock(Duration::from_secs(d.as_millis().div_ceil(1000) as u64)));
        let big = shown.unwrap_or_else(|| length.label());
        cv.numerals(&[(big.as_str(), Font::Timer, if on { Ink::Text } else { Ink::Dim })], 0.0, BASELINE, Align::Left);
        let status = match (on, remaining) {
            (true, Some(left)) => format!("Until {}", super::clock::time_after(left)),
            (true, None) => "Until you turn it off".into(),
            (false, _) => "Off · your PC sleeps as usual".into(),
        };
        cv.text(&status, Font::Small, Ink::Dim, Rect::new(0.0, 51.0, w, 18.0), Align::Left);

        for (i, (l, label), r) in chips() {
            let chosen = l == length;
            let lit = cv.ease(motion::key(("awake-chip", i)), f32::from(u8::from(chosen)), timing::STATE);
            cv.fill(r, space::CONTROL_RADIUS, Ink::Track);
            cv.fill_a(r, space::CONTROL_RADIUS, Ink::Hover, cv.hover_if(r, !chosen));
            cv.fill_a(r, space::CONTROL_RADIUS, Ink::Lit, lit);
            cv.outline(r, space::CONTROL_RADIUS, Ink::LitEdge, lit);
            cv.text(label, Font::Label, if chosen { Ink::Text } else { Ink::Dim }, r, Align::Center);
        }
        let track = track(w, h);
        slider(cv, track, length.fraction(), cv.hovered(slider_hit(track)), true);
        let value = Rect::new(w - VALUE_W, track.y - 9.0, VALUE_W, 20.0);
        cv.text(&length.label(), Font::Small, Ink::Text, value, Align::Right);
    }
}

impl Widget for AwakeWidget {
    fn title(&self) -> &'static str {
        "Awake"
    }

    fn icon(&self) -> &'static str {
        glyph::EYE
    }

    /// The shell ticks `Awake` itself, since the rail and the menu use it too.
    fn update(&mut self, _now: Instant) -> bool {
        false
    }

    /// None: the rail shows an eye while it's on instead (see `rail_eye`).
    fn compact(&self) -> Option<Compact> {
        None
    }

    fn draw(&self, cv: &Canvas, w: f32, h: f32) {
        let now = Instant::now();
        let [eye_card, length_card] = cards(w, h);
        self.draw_eye_card(cv, eye_card, now);
        cv.card(length_card, space::CARD_RADIUS, 0.0);
        let r = length_card.inset(space::CARD_PAD);
        self.draw_length(&cv.offset(r.x, r.y), r.w, r.h, now);
    }

    fn click(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let now = Instant::now();
        let [eye_card, length_card] = cards(w, h);
        if eye_card.contains(x, y) {
            self.awake.toggle(now);
            return true;
        }
        let r = length_card.inset(space::CARD_PAD);
        match chips().find(|(_, _, c)| c.contains(x - r.x, y - r.y)) {
            Some((_, (length, _), _)) => self.awake.set_length(length, now),
            None => false,
        }
    }

    /// A press on the slider sets the length there and starts a drag.
    fn press(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        self.size = (w, h);
        let r = cards(w, h)[1].inset(space::CARD_PAD);
        if !slider_hit(track(r.w, r.h)).contains(x - r.x, y - r.y) {
            return false;
        }
        self.drag(x, y);
        true
    }

    fn drag(&mut self, x: f32, _y: f32) -> bool {
        let r = cards(self.size.0, self.size.1)[1].inset(space::CARD_PAD);
        let length = Length::at(slider_at(track(r.w, r.h), x - r.x));
        // While on, only a different length restarts the count.
        length != self.awake.length() && self.awake.set_length(length, Instant::now())
    }

    fn load(&mut self, settings: &Settings) {
        self.awake.set_length(settings.awake_length, Instant::now());
    }

    fn store(&self, settings: &mut Settings) {
        settings.awake_length = self.awake.length();
    }

    /// While on: the countdown, the eye's blinking and its glow.
    fn animating(&self) -> bool {
        self.awake.on()
    }
}

/// The eye's card and the length's card, side by side.
fn cards(w: f32, h: f32) -> [Rect; 2] {
    let length_x = EYE_CARD_W + space::GAP;
    [Rect::new(0.0, 0.0, EYE_CARD_W, h), Rect::new(length_x, 0.0, w - length_x, h)]
}

/// Length chips under the status line, in the length card's content coordinates.
fn chips() -> impl Iterator<Item = (usize, (Length, &'static str), Rect)> {
    PRESETS.into_iter().enumerate().map(|(i, p)| (i, p, Rect::new(i as f32 * (CHIP_W + CHIP_GAP), 81.0, CHIP_W, CHIP_H)))
}

/// The slider's track along the bottom of the length card, leaving room for its value.
fn track(w: f32, h: f32) -> Rect {
    let x = 6.0;
    Rect::new(x, h - 10.0, w - x - VALUE_W - 12.0, 3.0)
}

/// How open a blinking eye is now, 0..=1: open, but every few seconds it closes and opens
/// again in a moment. Always open with animations off.
fn blink() -> f32 {
    if motion::reduced() {
        return 1.0;
    }
    let t = motion::cycle(BLINK_EVERY) * BLINK_EVERY / BLINK;
    if t < 1.0 { 0.06 + 0.94 * (2.0 * t - 1.0).abs() } else { 1.0 }
}

/// Whether a blink is under way (or just ended), so the rail's eye gets the frames for it.
pub fn blinking() -> bool {
    !motion::reduced() && motion::cycle(BLINK_EVERY) * BLINK_EVERY < BLINK + 0.05
}

/// The collapsed rail's eye, `size` px across, centred on `at`: open and blinking in cyan
/// while Awake is on; closed and dim (`open` false) beside the note on why it went off.
pub fn rail_eye(cv: &Canvas, at: (f32, f32), size: f32, open: bool) {
    if open {
        cv.halo(at.0, at.1, size * 0.3, Ink::Accent, 0.5);
        eye(cv, at, size, blink(), Ink::Accent);
    } else {
        eye(cv, at, size, 0.0, Ink::Dim);
    }
}

/// An eye `size` px across, centred on `at`, in `ink`. `open` (0..=1) runs from a closed lid,
/// lashes down, to an almond with its iris: in between, the almond is squashed shut and the
/// closed lid fades in.
fn eye(cv: &Canvas, (cx, cy): (f32, f32), size: f32, open: f32, ink: Ink) {
    let (a, open) = (size / 2.0, open.clamp(0.0, 1.0));
    let stroke = (size / 24.0).clamp(1.25, 3.0);
    if open < 1.0 {
        // The closed lid: a shallow arc sagging below the eye's middle line.
        let fade = 1.0 - open;
        let sag = a * 0.32;
        let r = (a * a + sag * sag) / (2.0 * sag);
        let lid = Path::new().move_to(cx - a, cy).arc_to(cx + a, cy, (r, r), false, false);
        if let Some(shape) = cv.path_shape(&lid) {
            cv.stroke_shape(&shape, ink, stroke, fade);
        }
        // Lashes, square to the lid, where they're big enough to see.
        if size >= 24.0 {
            let centre = cy + sag - r;
            for t in [-0.55, 0.0, 0.55] {
                let dx = t * a;
                let y = centre + (r * r - dx * dx).sqrt();
                let (nx, ny) = (dx / r, (y - centre) / r);
                let (from, to) = (stroke * 1.2, stroke * 1.2 + size * 0.11);
                cv.stroke_line((cx + dx + nx * from, y + ny * from), (cx + dx + nx * to, y + ny * to), ink, stroke, fade);
            }
        }
    }
    if open <= 0.0 {
        return;
    }
    // The open eye: two arcs meeting in points at either side.
    let b = a * 0.6;
    let r = (a * a + b * b) / (2.0 * b);
    let almond = Path::new()
        .move_to(cx - a, cy)
        .arc_to(cx + a, cy, (r, r), false, true)
        .arc_to(cx - a, cy, (r, r), false, true)
        .close();
    cv.squashed(cy, open, || {
        let Some(shape) = cv.path_shape(&almond) else { return };
        cv.fill_shape(&shape, ink, 0.12 * open);
        cv.masked(&shape, || {
            let iris = b * 0.92;
            cv.circle_a(cx, cy, iris, ink, None, 0.9 * open);
            cv.circle_a(cx, cy, iris * 0.45, Ink::Rgb(color::GLASS), None, open);
            cv.circle_a(cx + iris * 0.3, cy - iris * 0.32, iris * 0.16, Ink::Text, None, 0.9 * open);
        });
        cv.stroke_shape(&shape, ink, stroke, open);
    });
}

#[cfg(test)]
mod tests {
    use super::{CHIP_H, EYE_CARD_W, EYE_SIZE, PRESETS, VALUE_W, cards, chips, track};
    use crate::awake::Length;

    #[test]
    fn presets_are_slider_stops() {
        for (length, _) in PRESETS {
            assert_eq!(Length::at(length.fraction()), length);
        }
    }

    #[test]
    fn cards_chips_and_slider_fit_the_content_area() {
        // The expanded notch's content area.
        let (w, h) = (692.0, 146.0);
        let [eye, length] = cards(w, h);
        assert_eq!((eye.w, length.right()), (EYE_CARD_W, w));
        assert!(length.x > eye.right(), "a gap between them");
        assert!(EYE_SIZE * 0.6 / 2.0 + 58.0 < h - 24.0 - 16.0, "the open eye clears the status line");
        let (iw, ih) = (length.w - 24.0, h - 24.0);
        let last = chips().last().map(|(_, _, r)| r).unwrap();
        assert!(last.right() < iw - VALUE_W, "the chips fit");
        let t = track(iw, ih);
        assert!(t.y > last.y + CHIP_H + 4.0, "the slider is under the chips");
        assert!(t.y + 8.0 <= ih, "the knob stays inside");
    }
}
