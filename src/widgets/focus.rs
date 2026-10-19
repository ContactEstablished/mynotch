//! The Time tab's Timer card (a countdown), which is also the timer card on Home.

use std::cell::Cell;
use std::time::{Duration, Instant};

use super::styles::{Dial, Face, Size, Style};
use super::{Compact, button, button_in, button_row, clock, pulse, pulsing, slider, slider_at, slider_hit};
use crate::gfx::{Align, Canvas, Font, Ink, Rect, glyph};
use crate::motion;
use crate::settings::Settings;
use crate::theme::{alpha, motion as timing, space};
use crate::timer::Timer;

/// Timer lengths offered as chips, in minutes.
const PRESETS: [u32; 3] = [15, 25, 45];
/// The slider's range and step, in minutes.
const SLIDER_MIN: u32 = 5;
const SLIDER_MAX: u32 = 180;
const SLIDER_STEP: u32 = 5;
const CHIP_W: f32 = 40.0;
const CHIP_H: f32 = 21.0;
const CHIP_GAP: f32 = 6.0;
/// Room right of the slider for its value.
const VALUE_W: f32 = 46.0;
/// The ring around the start/pause button, its centre this far in from the card's right.
const RING_R: f32 = 29.0;
const RING_W: f32 = 2.5;
const RING_IN: f32 = 34.0;
const SMALL_BUTTON: f32 = 22.0;
/// Where the time sits (its baseline), on the card and on Home.
const BASELINE: f32 = 46.0;
const CARD_BASELINE: f32 = 47.0;
/// How full the gallery's rings are drawn while the timer is stopped, to show them off.
const PREVIEW_LEFT: f32 = 0.7;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Control {
    Reset,
    Toggle,
}

/// A countdown timer: the time left in a ring around start/pause, reset, and the length
/// (as chips and a slider) while it's stopped.
pub struct Focus {
    timer: Timer,
    style: Style,
    /// Whole seconds last drawn, so a running timer redraws once per second.
    shown_secs: u64,
    attention: bool,
    /// It reached zero and nothing has been touched since.
    done: bool,
    /// When it last started (or resumed), for its one soft pulse.
    started: Option<Instant>,
    /// Drawn last frame mid-pulse, so the frame that ends it gets drawn too.
    was_pulsing: Cell<bool>,
}

impl Focus {
    pub fn new() -> Self {
        Self {
            timer: Timer::new(minutes(25)),
            style: Style::default(),
            shown_secs: u64::MAX,
            attention: false,
            done: false,
            started: None,
            was_pulsing: Cell::new(false),
        }
    }

    pub fn running(&self) -> bool {
        self.timer.is_running()
    }

    pub fn style(&self) -> Style {
        self.style
    }

    /// Returns true when it changed.
    pub fn set_style(&mut self, style: Style) -> bool {
        std::mem::replace(&mut self.style, style) != style
    }

    /// While a Retro Clock timer runs: the time shown a second before, and how far (0..1)
    /// its digits have flipped over since the time changed.
    fn flip(&self, now: Instant) -> Option<(String, f32)> {
        if self.style != Style::Retro || !self.timer.is_running() || motion::reduced() {
            return None;
        }
        let shown = self.remaining(now).as_secs();
        // The time shown changed when the time left fell to a whole second.
        let since = shown as f32 - self.timer.remaining(now).as_secs_f32();
        let t = since / timing::FLIP.as_secs_f32();
        let before = shown + 1;
        (t < 1.0 && before <= self.timer.length().as_secs()).then(|| (clock(Duration::from_secs(before)), t.max(0.0)))
    }

    /// The timer as the style gallery's tiles show it; stopped, their rings are part full
    /// to show them off.
    pub fn preview(&self, now: Instant) -> (String, f32, bool) {
        let left = if self.timer.is_idle() { PREVIEW_LEFT } else { self.left(now) };
        (clock(self.remaining(now)), left, self.timer.is_running())
    }

    /// Remaining time, rounded up to whole seconds (so it reads 25:00, not 24:59, at start).
    fn remaining(&self, now: Instant) -> Duration {
        Duration::from_secs(self.timer.remaining(now).as_millis().div_ceil(1000) as u64)
    }

    /// Fraction of the time still to go.
    fn left(&self, now: Instant) -> f32 {
        self.timer.remaining(now).as_secs_f32() / self.timer.length().as_secs_f32().max(1.0)
    }

    fn paused(&self) -> bool {
        !self.timer.is_running() && !self.timer.is_idle()
    }

    /// The length can change only while the timer is stopped.
    fn adjustable(&self) -> bool {
        self.timer.is_idle()
    }

    fn length_minutes(&self) -> u32 {
        (self.timer.length().as_secs() / 60) as u32
    }

    fn set_length_minutes(&mut self, m: u32) -> bool {
        let changed = m != self.length_minutes();
        self.timer.set_length(minutes(m));
        self.done &= !changed;
        changed
    }

    /// One line on where things stand.
    fn status(&self, now: Instant) -> String {
        if self.timer.is_running() {
            format!("Ends at {}", super::clock::time_after(self.remaining(now)))
        } else if self.paused() {
            "Paused".into()
        } else if self.done {
            "Time's up".into()
        } else {
            "Ready".into()
        }
    }

    fn press_control(&mut self, control: Control) -> bool {
        match control {
            Control::Reset if self.timer.is_idle() && !self.done => return false,
            Control::Reset => self.timer.reset(),
            Control::Toggle => {
                let now = Instant::now();
                self.timer.toggle(now);
                if self.timer.is_running() {
                    self.started = Some(now);
                }
            }
        }
        self.done = false;
        self.shown_secs = u64::MAX;
        true
    }

    /// Returns true when something shown changed (once a second while running, and every
    /// frame of the start pulse).
    pub fn update(&mut self, now: Instant) -> bool {
        if self.timer.tick(now) {
            (self.attention, self.done, self.shown_secs) = (true, true, u64::MAX);
            return true;
        }
        let secs = self.remaining(now).as_secs();
        let changed = secs != self.shown_secs;
        self.shown_secs = secs;
        let pulse = pulsing(self.started);
        changed || pulse || self.was_pulsing.replace(pulse)
    }

    pub fn compact(&self) -> Option<Compact> {
        let now = Instant::now();
        let time = clock(self.remaining(now));
        let done = Some(1.0 - self.left(now));
        if self.timer.is_running() {
            Some(Compact { priority: 20, text: format!("Timer  {time}"), live: true, progress: done })
        } else if self.paused() {
            // Still worth a glance over CPU stats, but a playing song outranks it.
            Some(Compact { priority: 5, text: format!("Paused  {time}"), live: false, progress: done })
        } else {
            None
        }
    }

    /// The time, in the timer's style, from (0, `baseline`).
    fn draw_time(&self, cv: &Canvas, baseline: f32, now: Instant) {
        let time = clock(self.remaining(now));
        let flip = self.flip(now);
        let face = Face { time: &time, flip: flip.as_ref().map(|(was, t)| (was.as_str(), *t)), running: self.running() };
        self.style.time(cv, &face, 0.0, baseline, Size::Card);
    }

    /// The ring: time left around the start/pause button, in the timer's style.
    fn draw_ring(&self, cv: &Canvas, toggle: Rect, now: Instant) {
        let (cx, cy) = (toggle.x + toggle.w / 2.0, toggle.y + toggle.h / 2.0);
        let hot = cv.hover(toggle);
        let on = cv.ease(motion::key(("ring", cx as i32)), if self.paused() { 0.55 } else { 1.0 }, timing::STATE);
        let dial = Dial { cx, cy, r: RING_R, width: RING_W, left: self.left(now), running: self.running(), hot, on };
        self.style.ring(cv, &dial);
        let accent = Ink::Rgb(self.style.accent());
        pulse(cv, cx, cy, RING_R, accent, self.started);
        // The button inside, in the style's colour.
        let inner = RING_R - 7.0;
        cv.circle_a(cx, cy, inner, accent, None, alpha::LIT);
        cv.circle_a(cx, cy, inner, accent, None, alpha::LIT * hot);
        cv.circle_a(cx, cy, inner - 0.5, accent, Some(1.0), alpha::LIT_EDGE * (0.7 + 0.3 * hot));
        let icon = if self.timer.is_running() { glyph::PAUSE } else { glyph::PLAY };
        cv.text(icon, Font::IconMedium, Ink::Text, toggle, Align::Center);
    }

    pub fn draw(&self, cv: &Canvas, w: f32, h: f32) {
        let now = Instant::now();
        self.was_pulsing.set(pulsing(self.started));
        super::time::header(cv, None, "Timer", Ink::Rgb(self.style.accent()), w);

        self.draw_time(cv, BASELINE, now);
        let status = Rect::new(0.0, 51.0, w - RING_IN - RING_R - 6.0, 18.0);
        self.style.status(cv, &self.status(now), status);

        for (control, r) in controls(w) {
            match control {
                Control::Toggle => self.draw_ring(cv, r, now),
                Control::Reset => {
                    let enabled = !self.timer.is_idle() || self.done;
                    button(cv, r, glyph::RESET, Font::IconCaption, enabled, false);
                }
            }
        }

        // Length: chips for the usual ones, a slider for anything else. Greyed out while
        // the timer is running or paused.
        let enabled = self.adjustable();
        let current = self.length_minutes();
        for (m, r) in chips() {
            let chosen = m == current;
            let lit = cv.ease(motion::key(("chip", m)), f32::from(u8::from(chosen)), timing::STATE);
            cv.fill(r, space::CONTROL_RADIUS, Ink::Track);
            cv.fill_a(r, space::CONTROL_RADIUS, Ink::Hover, cv.hover_if(r, enabled && !chosen));
            cv.fill_a(r, space::CONTROL_RADIUS, Ink::Lit, lit * if enabled { 1.0 } else { 0.5 });
            cv.outline(r, space::CONTROL_RADIUS, Ink::LitEdge, lit * if enabled { 1.0 } else { 0.4 });
            let ink = if enabled && chosen { Ink::Text } else { Ink::Dim };
            cv.text(&format!("{m}m"), Font::Label, ink, r, Align::Center);
        }
        let track = track(w, h);
        slider(cv, track, fraction(current), cv.hovered(slider_hit(track)), enabled);
        let value = Rect::new(w - VALUE_W, track.y - 9.0, VALUE_W, 20.0);
        cv.text(&format!("{current} min"), Font::Small, if enabled { Ink::Text } else { Ink::Dim }, value, Align::Right);
    }

    pub fn click(&mut self, x: f32, y: f32, w: f32, _h: f32) -> bool {
        if let Some((control, _)) = controls(w).into_iter().find(|&(c, r)| hit(c, r, x, y)) {
            return self.press_control(control);
        }
        if self.adjustable()
            && let Some((m, _)) = chips().find(|(_, r)| r.contains(x, y))
        {
            return self.set_length_minutes(m);
        }
        false
    }

    /// A press on the slider sets the length there and starts a drag.
    pub fn press(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        if !self.adjustable() || !slider_hit(track(w, h)).contains(x, y) {
            return false;
        }
        self.drag(x, w, h);
        true
    }

    /// Returns true when the length changed.
    pub fn drag(&mut self, x: f32, w: f32, h: f32) -> bool {
        self.adjustable() && self.set_length_minutes(minutes_at(slider_at(track(w, h), x)))
    }

    pub fn load(&mut self, settings: &Settings) {
        self.timer.set_length(minutes(settings.focus_minutes));
        self.style = settings.timer_style;
    }

    pub fn store(&self, settings: &mut Settings) {
        settings.focus_minutes = self.length_minutes();
        settings.timer_style = self.style;
    }

    pub fn take_attention(&mut self) -> bool {
        std::mem::take(&mut self.attention)
    }

    pub fn draw_card(&self, cv: &Canvas, w: f32, h: f32) {
        let now = Instant::now();
        self.was_pulsing.set(pulsing(self.started));
        let accent = self.style.accent();
        super::time::header(cv, None, "Timer", Ink::Rgb(accent), w);
        self.draw_time(cv, CARD_BASELINE, now);
        self.style.status(cv, &self.status(now), Rect::new(0.0, 51.0, w, 18.0));
        // A thin bar of the time left, in the timer's style.
        self.style.bar(cv, Rect::new(0.0, 76.0, w, 2.0), self.left(now), self.running());
        for (control, r) in card_controls(w, h) {
            match control {
                Control::Toggle => {
                    let icon = if self.timer.is_running() { glyph::PAUSE } else { glyph::PLAY };
                    pulse(cv, r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, Ink::Rgb(accent), self.started);
                    button_in(cv, r, icon, Font::Icon, true, Some(accent));
                }
                Control::Reset => {
                    let enabled = !self.timer.is_idle() || self.done;
                    button(cv, r, glyph::RESET, Font::IconCaption, enabled, false);
                }
            }
        }
    }

    pub fn click_card(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        match card_controls(w, h).into_iter().find(|&(c, r)| hit(c, r, x, y)) {
            Some((control, _)) => self.press_control(control),
            None => false,
        }
    }
}

fn minutes(m: u32) -> Duration {
    Duration::from_secs(u64::from(m) * 60)
}

/// The slider's position for a length, 0..=1.
fn fraction(m: u32) -> f32 {
    (m.clamp(SLIDER_MIN, SLIDER_MAX) - SLIDER_MIN) as f32 / (SLIDER_MAX - SLIDER_MIN) as f32
}

/// The length at slider position `k`, in whole steps.
fn minutes_at(k: f32) -> u32 {
    let steps = (k.clamp(0.0, 1.0) * ((SLIDER_MAX - SLIDER_MIN) / SLIDER_STEP) as f32).round() as u32;
    SLIDER_MIN + steps * SLIDER_STEP
}

/// Whether (`x`, `y`) hits `control` at `r`: the ring is hit inside its circle.
fn hit(control: Control, r: Rect, x: f32, y: f32) -> bool {
    let (dx, dy) = (x - (r.x + r.w / 2.0), y - (r.y + r.h / 2.0));
    match control {
        Control::Toggle => dx * dx + dy * dy <= (r.w / 2.0) * (r.w / 2.0),
        Control::Reset => r.contains(x, y),
    }
}

/// The ring (start/pause) at the card's right, with reset under it.
fn controls(w: f32) -> [(Control, Rect); 2] {
    let (cx, cy) = (w - RING_IN, 44.0);
    let toggle = Rect::new(cx - RING_R, cy - RING_R, RING_R * 2.0, RING_R * 2.0);
    let reset = Rect::new(cx - SMALL_BUTTON / 2.0, 80.0, SMALL_BUTTON, SMALL_BUTTON);
    [(Control::Reset, reset), (Control::Toggle, toggle)]
}

fn card_controls(w: f32, h: f32) -> [(Control, Rect); 2] {
    let [reset, toggle] = button_row(w / 2.0, h - 17.0, [28.0, 34.0], 14.0);
    [(Control::Reset, reset), (Control::Toggle, toggle)]
}

/// Length chips under the status line.
fn chips() -> impl Iterator<Item = (u32, Rect)> {
    PRESETS.into_iter().enumerate().map(|(i, m)| (m, Rect::new(i as f32 * (CHIP_W + CHIP_GAP), 81.0, CHIP_W, CHIP_H)))
}

/// The slider's track along the bottom of the card, leaving room for its value.
fn track(w: f32, h: f32) -> Rect {
    let x = 6.0;
    Rect::new(x, h - 10.0, w - x - VALUE_W - 12.0, 3.0)
}

#[cfg(test)]
mod tests {
    use super::{CHIP_H, RING_R, SLIDER_MAX, SLIDER_MIN, chips, controls, fraction, minutes_at, track};

    #[test]
    fn slider_snaps_to_five_minutes_across_its_range() {
        assert_eq!(minutes_at(0.0), SLIDER_MIN);
        assert_eq!(minutes_at(1.0), SLIDER_MAX);
        assert_eq!(minutes_at(-3.0), SLIDER_MIN);
        for k in [0.11, 0.37, 0.5, 0.92] {
            assert_eq!(minutes_at(k) % 5, 0, "{k}");
        }
        for m in (SLIDER_MIN..=SLIDER_MAX).step_by(5) {
            assert_eq!(minutes_at(fraction(m)), m, "the knob for {m} min reads back as {m}");
        }
    }

    #[test]
    fn controls_chips_and_slider_fit_the_card_without_overlapping() {
        // The Time tab's Timer card content: 34% of the 676 px left between the cards, less
        // padding.
        let (w, h) = (676.0 * 0.34 - 24.0, 122.0);
        let [(_, reset), (_, ring)] = controls(w);
        let last_chip = chips().last().map(|(_, r)| r).unwrap();
        assert!(last_chip.right() < reset.x, "chips stop short of reset");
        assert!(ring.x > 100.0 + 6.0, "the ring clears the time, even past an hour (\"2:30:00\")");
        assert!(ring.right() <= w && ring.y >= 0.0, "inside the card");
        assert!(reset.y > ring.y + RING_R * 2.0 + 5.0, "reset sits under the ring and its glow");
        let t = track(w, h);
        assert!(t.y > last_chip.y + CHIP_H && t.y - 7.0 > reset.y + reset.h - 1.0, "the slider is under everything");
        assert!(t.y + 8.0 <= h, "the knob stays inside");
    }
}
