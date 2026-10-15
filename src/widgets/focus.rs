//! The Time tab's focus card (a Pomodoro timer), which is also the focus card on Home.

use std::time::{Duration, Instant};

use super::{Compact, button, button_row, clock, slider, slider_at, slider_hit};
use crate::gfx::{Align, Canvas, Font, Ink, Rect, glyph};
use crate::settings::Settings;
use crate::timer::{Phase, SESSIONS_PER_CYCLE, Timer};

/// Focus-session lengths offered as chips, in minutes.
const PRESETS: [u32; 3] = [15, 25, 50];
/// The slider's range and step, in minutes.
const SLIDER_MIN: u32 = 5;
const SLIDER_MAX: u32 = 180;
const SLIDER_STEP: u32 = 5;
const CHIP_W: f32 = 56.0;
const CHIP_H: f32 = 24.0;
const CHIP_GAP: f32 = 8.0;
/// Room right of the slider for its value.
const VALUE_W: f32 = 56.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Control {
    Reset,
    Toggle,
    Skip,
}

/// Pomodoro focus timer: phase, time left and progress, controls, and the focus length (as
/// chips and a slider) while a session waits to start.
pub struct Focus {
    timer: Timer,
    /// Whole seconds last drawn, so a running timer redraws once per second.
    shown_secs: u64,
    attention: bool,
}

impl Focus {
    pub fn new() -> Self {
        Self { timer: Timer::new(minutes(25)), shown_secs: u64::MAX, attention: false }
    }

    /// Remaining time, rounded up to whole seconds (so it reads 25:00, not 24:59, at start).
    fn remaining(&self, now: Instant) -> Duration {
        Duration::from_secs(self.timer.remaining(now).as_millis().div_ceil(1000) as u64)
    }

    /// Fraction of the phase still to go.
    fn left(&self, now: Instant) -> f32 {
        self.timer.remaining(now).as_secs_f32() / self.timer.phase_len().as_secs_f32().max(1.0)
    }

    fn phase_ink(&self) -> Ink {
        if self.timer.phase() == Phase::Focus { Ink::Accent } else { Ink::Calm }
    }

    fn paused(&self) -> bool {
        !self.timer.is_running() && !self.timer.is_idle()
    }

    /// The focus length can change only while a focus session waits to start.
    fn adjustable(&self) -> bool {
        self.timer.is_idle() && self.timer.phase() == Phase::Focus
    }

    fn focus_minutes(&self) -> u32 {
        (self.timer.focus_len().as_secs() / 60) as u32
    }

    fn set_focus_minutes(&mut self, m: u32) -> bool {
        let changed = m != self.focus_minutes();
        self.timer.set_focus_len(minutes(m));
        changed
    }

    fn label(&self) -> String {
        let t = &self.timer;
        let paused = if self.paused() { " · paused" } else { "" };
        match (t.phase(), t.is_idle()) {
            (Phase::Focus, _) => format!("Focus · session {} of {SESSIONS_PER_CYCLE}{paused}", t.completed() + 1),
            (Phase::ShortBreak, true) => "Time for a short break".into(),
            (Phase::LongBreak, true) => "Time for a long break".into(),
            (Phase::ShortBreak, false) => format!("Short break{paused}"),
            (Phase::LongBreak, false) => format!("Long break{paused}"),
        }
    }

    /// Shorter label for the Home card.
    fn short_label(&self) -> String {
        let t = &self.timer;
        let paused = if self.paused() { " · paused" } else { "" };
        match t.phase() {
            Phase::Focus => format!("Focus · {} of {SESSIONS_PER_CYCLE}{paused}", t.completed() + 1),
            Phase::ShortBreak => format!("Short break{paused}"),
            Phase::LongBreak => format!("Long break{paused}"),
        }
    }

    fn icon(&self, control: Control, big: bool) -> (&'static str, Font) {
        match control {
            Control::Reset => (glyph::RESET, Font::Icon),
            Control::Skip => (glyph::NEXT, Font::Icon),
            Control::Toggle => {
                let icon = if self.timer.is_running() { glyph::PAUSE } else { glyph::PLAY };
                (icon, if big { Font::IconLarge } else { Font::IconMedium })
            }
        }
    }

    fn draw_controls(&self, cv: &Canvas, controls: &[(Control, Rect); 3], big: bool) {
        for &(control, r) in controls {
            let enabled = control != Control::Reset || !self.timer.is_idle();
            let (icon, font) = self.icon(control, big);
            button(cv, r, icon, font, enabled, control == Control::Toggle);
        }
    }

    fn press_control(&mut self, control: Control) -> bool {
        match control {
            Control::Reset if self.timer.is_idle() => return false,
            Control::Reset => self.timer.reset(),
            Control::Toggle => self.timer.toggle(Instant::now()),
            Control::Skip => self.timer.skip(),
        }
        self.shown_secs = u64::MAX;
        true
    }

    /// Returns true when the time shown changed (once a second while running).
    pub fn update(&mut self, now: Instant) -> bool {
        if self.timer.tick(now).is_some() {
            self.attention = true;
            self.shown_secs = u64::MAX;
            return true;
        }
        let secs = self.remaining(now).as_secs();
        let changed = secs != self.shown_secs;
        self.shown_secs = secs;
        changed
    }

    pub fn compact(&self) -> Option<Compact> {
        let time = clock(self.remaining(Instant::now()));
        if self.timer.is_running() {
            let name = if self.timer.phase() == Phase::Focus { "Focus" } else { "Break" };
            Some(Compact { priority: 20, text: format!("{name}  {time}"), live: true })
        } else if self.paused() {
            // Still worth a glance over CPU stats, but a playing song outranks it.
            Some(Compact { priority: 5, text: format!("Paused  {time}"), live: false })
        } else {
            None
        }
    }

    pub fn draw(&self, cv: &Canvas, w: f32, h: f32) {
        let now = Instant::now();
        cv.text(&self.label(), Font::Small, Ink::Dim, Rect::new(0.0, 0.0, w, 16.0), Align::Left);
        let controls = controls(w);
        let time = Rect::new(0.0, 18.0, controls[0].1.x - 8.0, 42.0);
        cv.text(&clock(self.remaining(now)), Font::Display, Ink::Text, time, Align::Left);
        self.draw_controls(cv, &controls, false);

        let bar = Rect::new(0.0, 68.0, w, 4.0);
        cv.fill(bar, 2.0, Ink::Track);
        cv.fill(Rect { w: (bar.w * self.left(now)).max(bar.h), ..bar }, 2.0, self.phase_ink());

        // Focus length: chips for the usual ones, a slider for anything else. Greyed out
        // while a session or break is under way.
        let enabled = self.adjustable();
        let current = self.focus_minutes();
        for (m, r) in chips() {
            if m == current {
                cv.fill(r, CHIP_H / 2.0, Ink::Track);
            } else if enabled && cv.hovered(r) {
                cv.fill(r, CHIP_H / 2.0, Ink::Hover);
            }
            let ink = if enabled && m == current { Ink::Text } else { Ink::Dim };
            cv.text(&format!("{m} min"), Font::Small, ink, r, Align::Center);
        }
        let track = track(w, h);
        slider(cv, track, fraction(current), cv.hovered(slider_hit(track)), enabled);
        let value = Rect::new(track.right() + 12.0, track.y - 10.0, VALUE_W, 24.0);
        cv.text(&format!("{current} min"), Font::Body, if enabled { Ink::Text } else { Ink::Dim }, value, Align::Left);
    }

    pub fn click(&mut self, x: f32, y: f32, w: f32, _h: f32) -> bool {
        if let Some(&(control, _)) = controls(w).iter().find(|(_, r)| r.contains(x, y)) {
            return self.press_control(control);
        }
        if self.adjustable()
            && let Some((m, _)) = chips().find(|(_, r)| r.contains(x, y))
        {
            return self.set_focus_minutes(m);
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
        self.adjustable() && self.set_focus_minutes(minutes_at(slider_at(track(w, h), x)))
    }

    pub fn load(&mut self, settings: &Settings) {
        self.timer.set_focus_len(minutes(settings.focus_minutes));
    }

    pub fn store(&self, settings: &mut Settings) {
        settings.focus_minutes = self.focus_minutes();
    }

    pub fn take_attention(&mut self) -> bool {
        std::mem::take(&mut self.attention)
    }

    pub fn draw_card(&self, cv: &Canvas, w: f32, h: f32) {
        let now = Instant::now();
        cv.text(&clock(self.remaining(now)), Font::Display, Ink::Text, Rect::new(0.0, 0.0, w, 42.0), Align::Left);
        cv.text(&self.short_label(), Font::Small, Ink::Dim, Rect::new(0.0, 42.0, w, 18.0), Align::Left);
        let bar = Rect::new(0.0, 72.0, w, 4.0);
        cv.fill(bar, 2.0, Ink::Track);
        cv.fill(Rect { w: (bar.w * self.left(now)).max(bar.h), ..bar }, 2.0, self.phase_ink());
        self.draw_controls(cv, &card_controls(w, h), false);
    }

    pub fn click_card(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        match card_controls(w, h).iter().find(|(_, r)| r.contains(x, y)) {
            Some(&(control, _)) => self.press_control(control),
            None => false,
        }
    }
}

fn minutes(m: u32) -> Duration {
    Duration::from_secs(u64::from(m) * 60)
}

/// The slider's position for a focus length, 0..=1.
fn fraction(m: u32) -> f32 {
    (m.clamp(SLIDER_MIN, SLIDER_MAX) - SLIDER_MIN) as f32 / (SLIDER_MAX - SLIDER_MIN) as f32
}

/// The focus length at slider position `k`, in whole steps.
fn minutes_at(k: f32) -> u32 {
    let steps = (k.clamp(0.0, 1.0) * ((SLIDER_MAX - SLIDER_MIN) / SLIDER_STEP) as f32).round() as u32;
    SLIDER_MIN + steps * SLIDER_STEP
}

/// Reset, start/pause and skip, right-aligned beside the time.
fn controls(w: f32) -> [(Control, Rect); 3] {
    let total = 30.0 + 36.0 + 30.0 + 2.0 * 6.0;
    let [reset, toggle, skip] = button_row(w - total / 2.0, 39.0, [30.0, 36.0, 30.0], 6.0);
    [(Control::Reset, reset), (Control::Toggle, toggle), (Control::Skip, skip)]
}

fn card_controls(w: f32, h: f32) -> [(Control, Rect); 3] {
    let [reset, toggle, skip] = button_row(w / 2.0, h - 18.0, [32.0, 36.0, 32.0], 12.0);
    [(Control::Reset, reset), (Control::Toggle, toggle), (Control::Skip, skip)]
}

/// Session-length chips under the progress bar.
fn chips() -> impl Iterator<Item = (u32, Rect)> {
    PRESETS.into_iter().enumerate().map(|(i, m)| (m, Rect::new(i as f32 * (CHIP_W + CHIP_GAP), 82.0, CHIP_W, CHIP_H)))
}

/// The slider's track, under the chips, leaving room for its value.
fn track(w: f32, h: f32) -> Rect {
    let x = 8.0;
    Rect::new(x, (h - 20.0).min(128.0) - 2.0, w - x - VALUE_W - 12.0, 4.0)
}

#[cfg(test)]
mod tests {
    use super::{SLIDER_MAX, SLIDER_MIN, fraction, minutes_at};

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
}
