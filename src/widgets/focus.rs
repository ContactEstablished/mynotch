use std::time::{Duration, Instant};

use super::{Compact, Widget};
use crate::gfx::{Align, Canvas, Font, Ink, Rect, glyph};
use crate::settings::Settings;
use crate::timer::{Phase, SESSIONS_PER_CYCLE, Timer};

/// Focus-session lengths offered as chips, in minutes.
const PRESETS: [u32; 3] = [15, 25, 50];
const RING_R: f32 = 34.0;
const RING_W: f32 = 5.0;
const BUTTON: f32 = 36.0;
const BUTTON_GAP: f32 = 6.0;
const TEXT_X: f32 = 96.0;
const CHIP_W: f32 = 40.0;
const CHIP_H: f32 = 20.0;

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

    fn remaining_secs(&self, now: Instant) -> u64 {
        self.timer.remaining(now).as_millis().div_ceil(1000) as u64
    }

    fn label(&self) -> String {
        let t = &self.timer;
        match (t.phase(), t.is_idle()) {
            (Phase::Focus, _) => {
                let session = format!("Focus · session {} of {SESSIONS_PER_CYCLE}", t.completed() + 1);
                if t.is_running() || t.is_idle() { session } else { format!("{session} · paused") }
            }
            (Phase::ShortBreak, true) => "Time for a short break".into(),
            (Phase::LongBreak, true) => "Time for a long break".into(),
            (Phase::ShortBreak, false) if t.is_running() => "Short break".into(),
            (Phase::LongBreak, false) if t.is_running() => "Long break".into(),
            (_, false) => "Break · paused".into(),
        }
    }
}

fn minutes(m: u32) -> Duration {
    Duration::from_secs(u64::from(m) * 60)
}

/// "25:00", "4:05".
fn clock(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Reset, start/pause and skip: right-aligned and vertically centred.
fn buttons(w: f32, h: f32) -> [(Control, Rect); 3] {
    let y = (h - BUTTON) / 2.0;
    let x = |i: f32| w - BUTTON - i * (BUTTON + BUTTON_GAP);
    [
        (Control::Reset, Rect::new(x(2.0), y, BUTTON, BUTTON)),
        (Control::Toggle, Rect::new(x(1.0), y, BUTTON, BUTTON)),
        (Control::Skip, Rect::new(x(0.0), y, BUTTON, BUTTON)),
    ]
}

/// Session-length chips under the label, shown while a focus session is waiting to start.
fn chips() -> impl Iterator<Item = (u32, Rect)> {
    PRESETS
        .into_iter()
        .enumerate()
        .map(|(i, m)| (m, Rect::new(TEXT_X + i as f32 * (CHIP_W + 6.0), 70.0, CHIP_W, CHIP_H)))
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
        let secs = self.remaining_secs(now);
        let changed = secs != self.shown_secs;
        self.shown_secs = secs;
        changed
    }

    fn compact(&self) -> Option<Compact> {
        let time = clock(self.remaining_secs(Instant::now()));
        if self.timer.is_running() {
            let name = if self.timer.phase() == Phase::Focus { "Focus" } else { "Break" };
            Some(Compact { priority: 20, text: format!("{name}  {time}"), live: true })
        } else if !self.timer.is_idle() {
            // Paused: still worth a glance over CPU stats, but a playing song outranks it.
            Some(Compact { priority: 5, text: format!("Paused  {time}"), live: false })
        } else {
            None
        }
    }

    fn draw(&self, cv: &Canvas, w: f32, h: f32) {
        let now = Instant::now();
        let t = &self.timer;
        let ink = if t.phase() == Phase::Focus { Ink::Accent } else { Ink::Calm };

        // Ring: empties as the phase runs. Dots inside: focus sessions done this cycle.
        let (cx, cy) = (RING_R + RING_W, h / 2.0);
        let left = t.remaining(now).as_secs_f32() / t.phase_len().as_secs_f32().max(1.0);
        cv.arc(cx, cy, RING_R, 1.0, RING_W, Ink::Track);
        cv.arc(cx, cy, RING_R, left, RING_W, ink);
        let x0 = cx - (SESSIONS_PER_CYCLE - 1) as f32 * 5.0;
        for i in 0..SESSIONS_PER_CYCLE {
            let done = if i < t.completed() { Ink::Accent } else { Ink::Track };
            cv.circle(x0 + i as f32 * 10.0, cy, 3.0, done, None);
        }

        let buttons = buttons(w, h);
        let tw = buttons[0].1.x - 12.0 - TEXT_X;
        let secs = self.remaining_secs(now);
        cv.text(&clock(secs), Font::Display, Ink::Text, Rect::new(TEXT_X, 6.0, tw, 42.0), Align::Left);
        cv.text(&self.label(), Font::Small, Ink::Dim, Rect::new(TEXT_X, 48.0, tw, 18.0), Align::Left);

        if t.is_idle() && t.phase() == Phase::Focus {
            for (m, r) in chips() {
                let selected = minutes(m) == t.focus_len();
                if selected {
                    cv.fill(r, CHIP_H / 2.0, Ink::Track);
                } else if cv.hovered(r) {
                    cv.fill(r, CHIP_H / 2.0, Ink::Hover);
                }
                let text_ink = if selected { Ink::Text } else { Ink::Dim };
                cv.text(&format!("{m}m"), Font::Small, text_ink, r, Align::Center);
            }
        }

        for (control, r) in buttons {
            let on = control != Control::Reset || !t.is_idle();
            let (bx, by) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let hot = on && cv.hovered(r);
            let (icon, font) = match control {
                Control::Reset => (glyph::RESET, Font::Icon),
                Control::Skip => (glyph::NEXT, Font::Icon),
                Control::Toggle if t.is_running() => (glyph::PAUSE, Font::IconMedium),
                Control::Toggle => (glyph::PLAY, Font::IconMedium),
            };
            if control == Control::Toggle {
                cv.circle(bx, by, r.w / 2.0, if hot { Ink::Hover } else { Ink::Track }, None);
            } else if hot {
                cv.circle(bx, by, r.w / 2.0, Ink::Track, None);
            }
            cv.text(icon, font, if on { Ink::Text } else { Ink::Track }, r, Align::Center);
        }
    }

    fn click(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        if let Some((control, _)) = buttons(w, h).into_iter().find(|(_, r)| r.contains(x, y)) {
            match control {
                Control::Reset if self.timer.is_idle() => return false,
                Control::Reset => self.timer.reset(),
                Control::Toggle => self.timer.toggle(Instant::now()),
                Control::Skip => self.timer.skip(),
            }
            self.shown_secs = u64::MAX;
            return true;
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
}

#[cfg(test)]
mod tests {
    use super::clock;

    #[test]
    fn clock_format() {
        assert_eq!(clock(25 * 60), "25:00");
        assert_eq!(clock(245), "4:05");
        assert_eq!(clock(0), "0:00");
    }
}
