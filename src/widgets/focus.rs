use std::time::{Duration, Instant};

use super::{Compact, Widget, button, button_row, clock};
use crate::gfx::{Align, Canvas, Font, Ink, Rect, glyph};
use crate::settings::Settings;
use crate::timer::{Phase, SESSIONS_PER_CYCLE, Timer};

/// Focus-session lengths offered as chips, in minutes.
const PRESETS: [u32; 3] = [15, 25, 50];
const RING_R: f32 = 68.0;
const RING_W: f32 = 8.0;
const TEXT_X: f32 = 184.0;
const CHIP_W: f32 = 52.0;
const CHIP_H: f32 = 26.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Control {
    Reset,
    Toggle,
    Skip,
}

/// Pomodoro focus timer: a ring that empties as the phase runs, with session dots inside.
pub struct FocusWidget {
    timer: Timer,
    /// Whole seconds last drawn, so a running timer redraws once per second.
    shown_secs: u64,
    attention: bool,
}

impl FocusWidget {
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

    fn press(&mut self, control: Control) -> bool {
        match control {
            Control::Reset if self.timer.is_idle() => return false,
            Control::Reset => self.timer.reset(),
            Control::Toggle => self.timer.toggle(Instant::now()),
            Control::Skip => self.timer.skip(),
        }
        self.shown_secs = u64::MAX;
        true
    }
}

fn minutes(m: u32) -> Duration {
    Duration::from_secs(u64::from(m) * 60)
}

/// Reset, start/pause and skip, right-aligned and vertically centred.
fn controls(w: f32, h: f32) -> [(Control, Rect); 3] {
    let total = 40.0 + 48.0 + 40.0 + 2.0 * 12.0;
    let [reset, toggle, skip] = button_row(w - total / 2.0, h / 2.0, [40.0, 48.0, 40.0], 12.0);
    [(Control::Reset, reset), (Control::Toggle, toggle), (Control::Skip, skip)]
}

fn card_controls(w: f32, h: f32) -> [(Control, Rect); 3] {
    let [reset, toggle, skip] = button_row(w / 2.0, h - 18.0, [32.0, 36.0, 32.0], 12.0);
    [(Control::Reset, reset), (Control::Toggle, toggle), (Control::Skip, skip)]
}

/// Session-length chips under the label, shown while a focus session is waiting to start.
fn chips() -> impl Iterator<Item = (u32, Rect)> {
    PRESETS
        .into_iter()
        .enumerate()
        .map(|(i, m)| (m, Rect::new(TEXT_X + i as f32 * (CHIP_W + 8.0), 112.0, CHIP_W, CHIP_H)))
}

impl Widget for FocusWidget {
    fn title(&self) -> &'static str {
        "Focus"
    }

    fn update(&mut self, now: Instant) -> bool {
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

    fn compact(&self) -> Option<Compact> {
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

    fn draw(&self, cv: &Canvas, w: f32, h: f32) {
        let now = Instant::now();
        let t = &self.timer;

        // Ring: empties as the phase runs. Dots inside: focus sessions done this cycle.
        let (cx, cy) = (RING_R + RING_W / 2.0 + 4.0, h / 2.0);
        cv.arc(cx, cy, RING_R, 1.0, RING_W, Ink::Track);
        cv.arc(cx, cy, RING_R, self.left(now), RING_W, self.phase_ink());
        let x0 = cx - (SESSIONS_PER_CYCLE - 1) as f32 * 7.0;
        for i in 0..SESSIONS_PER_CYCLE {
            let done = if i < t.completed() { Ink::Accent } else { Ink::Track };
            cv.circle(x0 + i as f32 * 14.0, cy, 4.0, done, None);
        }

        let controls = controls(w, h);
        let tw = controls[0].1.x - 16.0 - TEXT_X;
        cv.text(&clock(self.remaining(now)), Font::Hero, Ink::Text, Rect::new(TEXT_X, 16.0, tw, 60.0), Align::Left);
        cv.text(&self.label(), Font::Body, Ink::Dim, Rect::new(TEXT_X, 78.0, tw, 20.0), Align::Left);

        if t.is_idle() && t.phase() == Phase::Focus {
            for (m, r) in chips() {
                let selected = minutes(m) == t.focus_len();
                if selected {
                    cv.fill(r, CHIP_H / 2.0, Ink::Track);
                } else if cv.hovered(r) {
                    cv.fill(r, CHIP_H / 2.0, Ink::Hover);
                }
                let ink = if selected { Ink::Text } else { Ink::Dim };
                cv.text(&format!("{m} min"), Font::Small, ink, r, Align::Center);
            }
        }
        self.draw_controls(cv, &controls, true);
    }

    fn click(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        if let Some(&(control, _)) = controls(w, h).iter().find(|(_, r)| r.contains(x, y)) {
            return self.press(control);
        }
        if self.timer.is_idle()
            && self.timer.phase() == Phase::Focus
            && let Some((m, _)) = chips().find(|(_, r)| r.contains(x, y))
        {
            self.timer.set_focus_len(minutes(m));
            return true;
        }
        false
    }

    fn load(&mut self, settings: &Settings) {
        self.timer.set_focus_len(minutes(settings.focus_minutes));
    }

    fn store(&self, settings: &mut Settings) {
        settings.focus_minutes = (self.timer.focus_len().as_secs() / 60) as u32;
    }

    fn take_attention(&mut self) -> bool {
        std::mem::take(&mut self.attention)
    }

    fn card_weight(&self) -> Option<f32> {
        Some(1.0)
    }

    fn draw_card(&self, cv: &Canvas, w: f32, h: f32) {
        let now = Instant::now();
        cv.text(&clock(self.remaining(now)), Font::Display, Ink::Text, Rect::new(0.0, 0.0, w, 42.0), Align::Left);
        cv.text(&self.short_label(), Font::Small, Ink::Dim, Rect::new(0.0, 42.0, w, 18.0), Align::Left);
        let bar = Rect::new(0.0, 72.0, w, 4.0);
        cv.fill(bar, 2.0, Ink::Track);
        cv.fill(Rect { w: (bar.w * self.left(now)).max(bar.h), ..bar }, 2.0, self.phase_ink());
        self.draw_controls(cv, &card_controls(w, h), false);
    }

    fn click_card(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        match card_controls(w, h).iter().find(|(_, r)| r.contains(x, y)) {
            Some(&(control, _)) => self.press(control),
            None => false,
        }
    }
}
