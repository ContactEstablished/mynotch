//! The Time tab's stopwatch card: start, pause, reset.

use std::time::{Duration, Instant};

use super::{Compact, button, button_row, clock};
use crate::gfx::{Align, Canvas, Font, Ink, Rect, glyph};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Control {
    Reset,
    Toggle,
}

/// A stopwatch counting up in tenths of a second. Time is passed in, for testing.
pub struct Stopwatch {
    /// Time counted by earlier runs.
    banked: Duration,
    /// When the current run started.
    since: Option<Instant>,
    /// Tenths last drawn, so a running stopwatch redraws ten times a second.
    shown: u128,
}

impl Stopwatch {
    pub fn new() -> Self {
        Self { banked: Duration::ZERO, since: None, shown: u128::MAX }
    }

    fn elapsed(&self, now: Instant) -> Duration {
        self.banked + self.since.map_or(Duration::ZERO, |s| now.saturating_duration_since(s))
    }

    fn is_running(&self) -> bool {
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

    /// Returns true when the time shown changed.
    pub fn update(&mut self, now: Instant) -> bool {
        let tenths = self.elapsed(now).as_millis() / 100;
        let changed = tenths != self.shown;
        self.shown = tenths;
        changed
    }

    pub fn compact(&self) -> Option<Compact> {
        let now = Instant::now();
        let time = clock(self.elapsed(now));
        // Below a running focus timer and a playing song; a paused one, just above CPU stats.
        if self.is_running() {
            Some(Compact { priority: 8, text: format!("Stopwatch  {time}"), live: true })
        } else if self.elapsed(now) > Duration::ZERO {
            Some(Compact { priority: 4, text: format!("Stopwatch  {time}"), live: false })
        } else {
            None
        }
    }

    pub fn draw(&self, cv: &Canvas, w: f32, h: f32) {
        let now = Instant::now();
        let label = if !self.is_running() && self.elapsed(now) > Duration::ZERO { "Stopwatch · paused" } else { "Stopwatch" };
        cv.text(label, Font::Small, Ink::Dim, Rect::new(0.0, 0.0, w, 16.0), Align::Left);
        cv.text(&format_elapsed(self.elapsed(now)), Font::Display, Ink::Text, Rect::new(0.0, 18.0, w, 42.0), Align::Left);
        for (control, r) in controls(w, h) {
            match control {
                Control::Reset => {
                    button(cv, r, glyph::RESET, Font::Icon, self.elapsed(now) > Duration::ZERO, false);
                }
                Control::Toggle => {
                    let icon = if self.is_running() { glyph::PAUSE } else { glyph::PLAY };
                    button(cv, r, icon, Font::IconLarge, true, true);
                }
            }
        }
    }

    pub fn click(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        match controls(w, h).into_iter().find(|(_, r)| r.contains(x, y)) {
            Some((Control::Toggle, _)) => self.toggle(Instant::now()),
            Some((Control::Reset, _)) if self.elapsed(Instant::now()) > Duration::ZERO => self.reset(),
            _ => return false,
        }
        self.shown = u128::MAX;
        true
    }
}

/// Reset and start/pause, centred at the bottom.
fn controls(w: f32, h: f32) -> [(Control, Rect); 2] {
    let [reset, toggle] = button_row(w / 2.0, h - 26.0, [36.0, 48.0], 16.0);
    [(Control::Reset, reset), (Control::Toggle, toggle)]
}

/// "0:07.3", "12:34.5", then "1:02:03" past an hour.
fn format_elapsed(d: Duration) -> String {
    if d.as_secs() >= 3600 {
        clock(d)
    } else {
        format!("{}.{}", clock(d), d.subsec_millis() / 100)
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
        assert_eq!(format_elapsed(Duration::from_millis(7_340)), "0:07.3");
        assert_eq!(format_elapsed(Duration::from_millis(754_560)), "12:34.5");
        assert_eq!(format_elapsed(Duration::from_secs(3723)), "1:02:03");
    }
}
