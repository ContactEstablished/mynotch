//! The Time tab's stopwatch card: start, pause, reset.

use std::cell::Cell;
use std::time::{Duration, Instant};

use super::{Compact, button, clock, pill, pulsing};
use crate::gfx::{Align, Canvas, Font, Ink, Rect, glyph};
use crate::motion::{self, reduced};
use crate::theme::{motion as timing, space};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Control {
    Reset,
    Toggle,
}

/// Wide enough for "Resume".
const PILL_W: f32 = 88.0;
const CONTROL_H: f32 = 28.0;
/// Where the time sits (its baseline).
const BASELINE: f32 = 56.0;

/// A stopwatch counting up in tenths of a second. Time is passed in, for testing.
pub struct Stopwatch {
    /// Time counted by earlier runs.
    banked: Duration,
    /// When the current run started.
    since: Option<Instant>,
    /// Tenths last drawn, so a running stopwatch redraws ten times a second.
    shown: u128,
    /// When it last started (or resumed), for its one soft pulse.
    started: Option<Instant>,
    /// Drawn last frame mid-pulse, so the frame that ends it gets drawn too.
    was_pulsing: Cell<bool>,
}

impl Stopwatch {
    pub fn new() -> Self {
        Self { banked: Duration::ZERO, since: None, shown: u128::MAX, started: None, was_pulsing: Cell::new(false) }
    }

    fn elapsed(&self, now: Instant) -> Duration {
        self.banked + self.since.map_or(Duration::ZERO, |s| now.saturating_duration_since(s))
    }

    pub fn is_running(&self) -> bool {
        self.since.is_some()
    }

    /// Start, pause or resume.
    fn toggle(&mut self, now: Instant) {
        match self.since.take() {
            Some(since) => self.banked += now.saturating_duration_since(since),
            None => self.since = Some(now),
        }
    }

    /// Stop and go back to zero.
    fn reset(&mut self) {
        self.banked = Duration::ZERO;
        self.since = None;
    }

    /// Returns true when the time shown changed, and every frame of the start pulse.
    pub fn update(&mut self, now: Instant) -> bool {
        let tenths = self.elapsed(now).as_millis() / 100;
        let changed = tenths != self.shown;
        self.shown = tenths;
        let pulse = pulsing(self.started);
        changed || pulse || self.was_pulsing.replace(pulse)
    }

    pub fn compact(&self) -> Option<Compact> {
        let now = Instant::now();
        let time = clock(self.elapsed(now));
        // Below a running focus timer and a playing song; a paused one, just above CPU stats.
        if self.is_running() {
            Some(Compact { priority: 8, text: format!("Stopwatch  {time}"), live: true, progress: None })
        } else if self.elapsed(now) > Duration::ZERO {
            Some(Compact { priority: 4, text: format!("Stopwatch  {time}"), live: false, progress: None })
        } else {
            None
        }
    }

    pub fn draw(&self, cv: &Canvas, w: f32, h: f32) {
        let now = Instant::now();
        self.was_pulsing.set(pulsing(self.started));
        let elapsed = self.elapsed(now);
        super::time::header(cv, Some(glyph::STOPWATCH), "Stopwatch", Ink::Dim, w);
        if !self.is_running() && elapsed > Duration::ZERO {
            cv.caps("Paused", Font::Caption, Ink::Dim, Rect::new(0.0, -1.0, w, 16.0), Align::Right);
        }
        let (whole, tenths) = format_elapsed(elapsed);
        let runs = [(whole.as_str(), Font::Stopwatch, Ink::Text), (tenths.as_str(), Font::Fraction, Ink::Dim)];
        cv.numerals(&runs, 0.0, BASELINE, Align::Left);
        // This minute, swept by a light like a second hand: moving while it runs, dimmed
        // while paused.
        let y = 70.0;
        cv.line((0.0, y), (w, y), Ink::Line);
        if elapsed > Duration::ZERO {
            let x = (w * (elapsed.as_secs_f32() % 60.0) / 60.0).max(1.5);
            let a = if self.is_running() { 1.0 } else { 0.45 };
            cv.fill_a(Rect::new(0.0, y - 0.75, x, 1.5), 0.75, Ink::Accent, 0.8 * a);
            if self.is_running() {
                let swell = if reduced() { 0.6 } else { motion::breath(2.0) };
                cv.halo(x, y, 2.0 + swell, Ink::Accent, 0.35 + 0.65 * swell);
            }
            cv.circle_a(x, y, 2.0, Ink::Accent, None, a);
        }
        for (control, r) in controls(w, h) {
            match control {
                Control::Reset => button(cv, r, glyph::RESET, Font::Icon, elapsed > Duration::ZERO, false),
                Control::Toggle => {
                    let (icon, label) = match (self.is_running(), elapsed > Duration::ZERO) {
                        (true, _) => (glyph::PAUSE, "Pause"),
                        (false, true) => (glyph::PLAY, "Resume"),
                        (false, false) => (glyph::PLAY, "Start"),
                    };
                    let lit = cv.ease(motion::key("stopwatch"), f32::from(u8::from(self.is_running())), timing::STATE);
                    self.draw_pulse(cv, r);
                    pill(cv, r, icon, label, lit);
                }
            }
        }
    }

    /// One soft glow spreading out from the Start button as the stopwatch starts.
    fn draw_pulse(&self, cv: &Canvas, r: Rect) {
        let Some(since) = self.started.filter(|_| !reduced()) else { return };
        let t = since.elapsed().as_secs_f32() / timing::PULSE.as_secs_f32();
        if t < 1.0 {
            let grow = 10.0 * (1.0 - (1.0 - t) * (1.0 - t));
            cv.outline(r.inset(-grow), space::CONTROL_RADIUS + grow, Ink::Accent, 0.45 * (1.0 - t));
        }
    }

    pub fn click(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        match controls(w, h).into_iter().find(|(_, r)| r.contains(x, y)) {
            Some((Control::Toggle, _)) => {
                let now = Instant::now();
                self.toggle(now);
                if self.is_running() {
                    self.started = Some(now);
                }
            }
            Some((Control::Reset, _)) if self.elapsed(Instant::now()) > Duration::ZERO => self.reset(),
            _ => return false,
        }
        self.shown = u128::MAX;
        true
    }
}

/// Start/pause as a labelled button, then reset, along the bottom.
fn controls(_w: f32, h: f32) -> [(Control, Rect); 2] {
    let y = h - CONTROL_H - 6.0;
    let toggle = Rect::new(0.0, y, PILL_W, CONTROL_H);
    let reset = Rect::new(toggle.right() + 6.0, y, CONTROL_H, CONTROL_H);
    [(Control::Reset, reset), (Control::Toggle, toggle)]
}

/// ("00:07", ".3"), ("12:34", ".5"), then ("1:02:03", "") past an hour.
fn format_elapsed(d: Duration) -> (String, String) {
    let s = d.as_secs();
    if s >= 3600 {
        (clock(d), String::new())
    } else {
        (format!("{:02}:{:02}", s / 60, s % 60), format!(".{}", d.subsec_millis() / 100))
    }
}

#[cfg(test)]
mod tests {
    use super::{Stopwatch, format_elapsed};
    use std::time::{Duration, Instant};

    #[test]
    fn counts_only_while_running() {
        let t0 = Instant::now();
        let at = |s: f32| t0 + Duration::from_secs_f32(s);
        let mut sw = Stopwatch::new();
        assert_eq!(sw.elapsed(at(5.0)), Duration::ZERO);
        sw.toggle(at(1.0));
        assert_eq!(sw.elapsed(at(3.5)), Duration::from_secs_f32(2.5));
        sw.toggle(at(4.0));
        assert!(!sw.is_running());
        assert_eq!(sw.elapsed(at(60.0)), Duration::from_secs(3), "paused: holds");
        sw.toggle(at(10.0));
        assert_eq!(sw.elapsed(at(12.0)), Duration::from_secs(5), "resumed: adds on");
        sw.reset();
        assert!(!sw.is_running());
        assert_eq!(sw.elapsed(at(20.0)), Duration::ZERO);
    }

    #[test]
    fn redraws_on_each_tenth() {
        let t0 = Instant::now();
        let mut sw = Stopwatch::new();
        assert!(sw.update(t0), "first draw");
        assert!(!sw.update(t0 + Duration::from_secs(1)), "idle: nothing changes");
        sw.toggle(t0);
        assert!(sw.update(t0 + Duration::from_millis(100)));
        assert!(!sw.update(t0 + Duration::from_millis(150)));
        assert!(sw.update(t0 + Duration::from_millis(200)));
    }

    #[test]
    fn format() {
        let parts = |d| { let (a, b) = format_elapsed(d); format!("{a}{b}") };
        assert_eq!(parts(Duration::from_millis(7_340)), "00:07.3");
        assert_eq!(parts(Duration::from_millis(754_560)), "12:34.5");
        assert_eq!(parts(Duration::from_secs(3723)), "1:02:03");
    }
}
