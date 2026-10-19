//! The notch's side of MyNotch Voice (the `mynotch-voice` crate does the work). While you
//! dictate, the notch hands itself over to a recording strip, collapsed or open: a red "REC"
//! dot, a live waveform of your voice, "Listening · Notepad" and a timer while Right Ctrl is
//! held; then "Transcribing…"; then "✓ Inserted into Notepad" for a moment, or what went wrong.
//! It shows over fullscreen apps too, and never opens the notch or takes the focus.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use mynotch_voice::audio::MAX_SECONDS;
use mynotch_voice::{Config, TAP, VoiceEvent, VoiceHandle, VoiceService};

use crate::gfx::{Align, Canvas, Font, Ink, Rect};
use crate::motion;
use crate::theme::color;

const INSERTED_FOR: Duration = Duration::from_millis(1500);
const FAILED_FOR: Duration = Duration::from_secs(3);
/// A session that's been quiet this long ended without saying so (voice stopped, say): stop
/// showing it rather than cover the notch for good.
const LISTENING_LIMIT: Duration = Duration::from_secs(MAX_SECONDS as u64 + 10);
const TRANSCRIBING_LIMIT: Duration = Duration::from_secs(60);
/// Levels kept for the waveform (about 2 s at 30 a second); it shows as many as fit.
const LEVELS: usize = 56;
/// Room for the dot and "REC" before the waveform.
const LEAD: f32 = 46.0;
/// The quietest and loudest the waveform shows, in dB below full scale.
const FLOOR_DB: f32 = -60.0;
const PEAK_DB: f32 = -20.0;
/// One sweep of the light across the waveform while transcribing.
const SWEEP: f32 = 0.9;

pub struct Voice {
    handle: VoiceHandle,
    rail: Rail,
}

impl Voice {
    pub fn start() -> Self {
        Self { handle: VoiceService::start(Config::from_env()), rail: Rail::default() }
    }

    /// Takes in what voice reported. Returns whether the notch needs a redraw.
    pub fn poll(&mut self, now: Instant) -> bool {
        self.rail.take(self.handle.events(), now)
    }

    /// Whether the recording strip has the notch.
    pub fn showing(&self) -> bool {
        self.rail.shown.is_some()
    }

    /// Something on the strip moves by itself: the dot breathing, the light sweeping.
    pub fn animating(&self) -> bool {
        matches!(self.rail.shown, Some(View::Listening { .. } | View::Transcribing { .. }))
    }

    /// The collapsed notch, `w` × `h`: the whole of it is the strip, with a glow along the
    /// bottom that follows your voice.
    pub fn draw_rail(&self, cv: &Canvas, w: f32, h: f32, radius: f32) {
        self.rail.draw(cv, Rect::new(14.0, 0.0, w - 28.0, h), false);
        self.rail.draw_glow(cv, radius, w - radius, h - 1.25);
    }

    /// The open notch's toolbar, in `r`, with the glow along `line_y` under it.
    pub fn draw_toolbar(&self, cv: &Canvas, r: Rect, line_y: f32) {
        self.rail.draw(cv, r, true);
        self.rail.draw_glow(cv, r.x, r.right(), line_y);
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Session {
    Idle,
    Listening { since: Instant, app: String },
    Transcribing { since: Instant, app: String },
}

/// What the strip shows.
#[derive(Clone, Debug, PartialEq)]
enum View {
    /// Elapsed whole seconds, for the timer.
    Listening { app: String, seconds: u64 },
    /// When it started, for the sweep.
    Transcribing { since: Instant },
    /// "Inserted into Notepad" (`ok`), or what went wrong.
    Note { text: String, ok: bool },
}

/// The strip's state, worked out from voice's events (kept apart from the service so it's
/// unit-tested).
#[derive(Debug)]
struct Rail {
    session: Session,
    /// The last result and when it stops showing. A new session's tap window, or a tap,
    /// leaves it up.
    note: Option<(String, bool, Instant)>,
    /// Recent loudness, 0..=1, newest last.
    levels: VecDeque<f32>,
    /// What showed at the last update, so a redraw is asked for only on a change.
    shown: Option<View>,
}

impl Default for Rail {
    fn default() -> Self {
        Self { session: Session::Idle, note: None, levels: VecDeque::with_capacity(LEVELS), shown: None }
    }
}

impl Rail {
    fn take(&mut self, events: impl IntoIterator<Item = VoiceEvent>, now: Instant) -> bool {
        let mut heard = false;
        for event in events {
            match event {
                VoiceEvent::SessionStarted { app, .. } => {
                    self.session = Session::Listening { since: now, app };
                    self.levels.clear();
                }
                VoiceEvent::Level { rms, .. } => {
                    if self.levels.len() == LEVELS {
                        self.levels.pop_front();
                    }
                    self.levels.push_back(loudness(rms));
                    heard = true;
                }
                VoiceEvent::Transcribing { .. } => {
                    let app = match &self.session {
                        Session::Listening { app, .. } => app.clone(),
                        _ => String::new(),
                    };
                    self.session = Session::Transcribing { since: now, app };
                }
                VoiceEvent::Transcript { .. } => {}
                VoiceEvent::Delivered { app, .. } => {
                    self.session = Session::Idle;
                    let text = if app.is_empty() { "Inserted".to_owned() } else { format!("Inserted into {app}") };
                    self.note = Some((text, true, now + INSERTED_FOR));
                }
                VoiceEvent::Cancelled { .. } => self.session = Session::Idle,
                VoiceEvent::Failed { error, .. } => {
                    self.session = Session::Idle;
                    self.note = Some((error, false, now + FAILED_FOR));
                }
            }
        }
        match self.session {
            Session::Listening { since, .. } if now - since > LISTENING_LIMIT => self.session = Session::Idle,
            Session::Transcribing { since, .. } if now - since > TRANSCRIBING_LIMIT => self.session = Session::Idle,
            _ => {}
        }
        if self.note.as_ref().is_some_and(|(_, _, until)| now >= *until) {
            self.note = None;
        }
        let shown = match &self.session {
            Session::Listening { since, app } if now - *since >= TAP => {
                self.note = None;
                Some(View::Listening { app: app.clone(), seconds: (now - *since).as_secs() })
            }
            Session::Transcribing { since, .. } => {
                self.note = None;
                Some(View::Transcribing { since: *since })
            }
            _ => self.note.as_ref().map(|(text, ok, _)| View::Note { text: text.clone(), ok: *ok }),
        };
        let changed = shown != self.shown || heard && matches!(shown, Some(View::Listening { .. }));
        self.shown = shown;
        changed
    }

    /// The strip in `r`: the dot (or the result's mark), the waveform (as many bars as fit),
    /// the words and the timer.
    fn draw(&self, cv: &Canvas, r: Rect, wide: bool) {
        let Some(view) = &self.shown else { return };
        let cy = r.y + r.h / 2.0;
        let pitch = if wide { 5.0 } else { 4.0 };
        // The waveform fills what the words and the timer leave, ending 12 px before the words.
        let wave = |cv: &Canvas, words: &str, timer_w: f32| {
            let room = r.w - LEAD - 12.0 - cv.measure(words, Font::Small) - timer_w;
            let bars = ((room / pitch).floor().max(0.0) as usize).min(LEVELS);
            (Rect::new(r.x + LEAD, r.y + r.h * 0.2, bars as f32 * pitch, r.h * 0.6), bars)
        };
        match view {
            View::Listening { app, seconds } => {
                let swell = if motion::reduced() { 0.6 } else { motion::breath(1.2) };
                cv.halo(r.x + 5.0, cy, 4.0, Ink::Rgb(color::RECORD), 0.4 + 0.6 * swell);
                cv.circle(r.x + 5.0, cy, 4.0, Ink::Rgb(color::RECORD), None);
                cv.caps("Rec", Font::Caption, Ink::Rgb(color::RECORD), Rect::new(r.x + 15.0, r.y, 30.0, r.h), Align::Left);
                let timer = format!("{}:{:02}", seconds / 60, seconds % 60);
                let timer_w = cv.numerals(&[(timer.as_str(), Font::Numeral, Ink::Dim)], r.right(), cy + 5.0, Align::Right);
                let words = if wide && !app.is_empty() { format!("Listening · {app}") } else { "Listening".to_owned() };
                let (rect, bars) = wave(cv, &words, timer_w + 10.0);
                self.draw_wave(cv, rect, bars, pitch, None);
                let at = rect.right() + 12.0;
                cv.text(&words, Font::Small, Ink::Text, Rect::new(at, r.y, r.right() - timer_w - 8.0 - at, r.h), Align::Left);
            }
            View::Transcribing { since } => {
                let sweep = if motion::reduced() { None } else { Some(since.elapsed().as_secs_f32() / SWEEP % 1.0) };
                let (rect, bars) = wave(cv, "Transcribing…", 0.0);
                self.draw_wave(cv, rect, bars, pitch, Some(sweep));
                let at = rect.right() + 12.0;
                cv.text("Transcribing…", Font::Small, Ink::Text, Rect::new(at, r.y, r.right() - at, r.h), Align::Left);
            }
            View::Note { text, ok } => {
                let (mark, ink) = if *ok { ("✓", Ink::Calm) } else { ("!", Ink::Rgb(color::WARN)) };
                cv.text(mark, Font::Small, ink, Rect::new(r.x, r.y, 12.0, r.h), Align::Center);
                let room = Rect::new(r.x + 18.0, r.y, r.w - 18.0, r.h);
                cv.text(text, Font::Small, if *ok { Ink::Text } else { ink }, room, Align::Left);
            }
        }
    }

    /// The newest `bars` levels as bars in `r`, oldest faintest, from the right (silence
    /// before the first ones, so the track is whole from the start). While transcribing
    /// (`sweep`), they rest dimmed with a light passing across them (at that fraction of the
    /// way, if moving).
    fn draw_wave(&self, cv: &Canvas, r: Rect, bars: usize, pitch: f32, sweep: Option<Option<f32>>) {
        let width = (pitch * 0.55).max(1.5);
        let cy = r.y + r.h / 2.0;
        let newest = self.levels.iter().rev().copied().chain(std::iter::repeat(0.0)).take(bars);
        for (i, level) in newest.enumerate() {
            let x = r.right() - (i as f32 + 1.0) * pitch;
            let h = (2.0 + level * (r.h - 2.0)).min(r.h);
            let bar = Rect::new(x, cy - h / 2.0, width, h);
            let age = 1.0 - i as f32 / bars as f32;
            match sweep {
                None => cv.fill_a(bar, width / 2.0, Ink::Accent, 0.35 + 0.65 * age),
                Some(at) => {
                    let place = 1.0 - (i as f32 + 0.5) / bars as f32;
                    let lit = at.map_or(0.0, |at| (1.0 - (place - at).abs() * 6.0).max(0.0));
                    cv.fill_a(bar, width / 2.0, Ink::Dim, 0.35);
                    cv.fill_a(bar, width / 2.0, Ink::Accent, lit);
                }
            }
        }
    }

    /// A glowing line from `x0` to `x1` at `y`: red, following the voice, while listening;
    /// cyan while transcribing.
    fn draw_glow(&self, cv: &Canvas, x0: f32, x1: f32, y: f32) {
        match self.shown {
            Some(View::Listening { .. }) => {
                let level = self.levels.back().copied().unwrap_or(0.0);
                let a = 0.35 + 0.65 * level;
                cv.stroke_line((x0, y), (x1, y), Ink::Rgb(color::RECORD), 4.0, 0.18 * a);
                cv.stroke_line((x0, y), (x1, y), Ink::Rgb(color::RECORD), 1.5, a);
            }
            Some(View::Transcribing { .. }) => cv.beam(x0, x1, y, 0.8),
            _ => {}
        }
    }
}

/// How loud an RMS level looks, 0..=1, on a dB scale from `FLOOR_DB` to `PEAK_DB`.
fn loudness(rms: f32) -> f32 {
    let db = 20.0 * rms.max(1e-6).log10();
    ((db - FLOOR_DB) / (PEAK_DB - FLOOR_DB)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn started(id: u64) -> VoiceEvent {
        VoiceEvent::SessionStarted { id, app: "Notepad".into() }
    }

    fn level(rms: f32) -> VoiceEvent {
        VoiceEvent::Level { id: 1, rms }
    }

    fn note(r: &Rail) -> Option<(&str, bool)> {
        match &r.shown {
            Some(View::Note { text, ok }) => Some((text.as_str(), *ok)),
            _ => None,
        }
    }

    #[test]
    fn listening_waits_out_a_tap_then_takes_the_notch() {
        let t0 = Instant::now();
        let mut r = Rail::default();
        assert!(!r.take([started(1)], t0), "nothing to show yet");
        assert!(!r.take([], t0 + TAP - ms(1)));
        assert!(r.take([], t0 + TAP), "the strip appears: redraw");
        assert_eq!(r.shown, Some(View::Listening { app: "Notepad".into(), seconds: 0 }));
        assert!(!r.take([], t0 + TAP + ms(16)), "nothing new, no redraw");
        assert!(r.take([], t0 + ms(1000)), "the timer ticks");
        assert_eq!(r.shown, Some(View::Listening { app: "Notepad".into(), seconds: 1 }));
    }

    #[test]
    fn levels_feed_the_waveform_and_redraw_it() {
        let t0 = Instant::now();
        let mut r = Rail::default();
        r.take([started(1)], t0);
        r.take([], t0 + TAP);
        assert!(r.take([level(0.05)], t0 + TAP + ms(33)), "a new level redraws");
        for _ in 0..LEVELS + 10 {
            r.take([level(0.01)], t0 + TAP + ms(66));
        }
        assert_eq!(r.levels.len(), LEVELS, "only the newest are kept");
        r.take([started(2)], t0 + ms(5000));
        assert!(r.levels.is_empty(), "a new session starts a new waveform");
    }

    #[test]
    fn loudness_is_on_a_db_scale() {
        assert_eq!(loudness(0.0), 0.0);
        assert_eq!(loudness(0.001), 0.0, "-60 dB: silence");
        assert!((loudness(0.01) - 0.5).abs() < 1e-4, "-40 dB: halfway");
        assert_eq!(loudness(0.1), 1.0, "-20 dB: full");
        assert_eq!(loudness(1.0), 1.0);
    }

    #[test]
    fn a_tap_never_shows() {
        let t0 = Instant::now();
        let mut r = Rail::default();
        r.take([started(1)], t0);
        assert!(!r.take([VoiceEvent::Cancelled { id: 1 }], t0 + ms(100)));
        assert!(!r.take([], t0 + ms(1000)));
        assert!(r.shown.is_none());
    }

    #[test]
    fn transcribing_then_inserted_into_the_app_for_a_moment() {
        let t0 = Instant::now();
        let mut r = Rail::default();
        r.take([started(1)], t0);
        r.take([VoiceEvent::Transcribing { id: 1 }], t0 + ms(2000));
        assert_eq!(r.shown, Some(View::Transcribing { since: t0 + ms(2000) }));
        let t1 = t0 + ms(2300);
        let done = VoiceEvent::Delivered { id: 1, app: "Notepad".into() };
        assert!(r.take([VoiceEvent::Transcript { id: 1, text: "Hi".into() }, done], t1));
        assert_eq!(note(&r), Some(("Inserted into Notepad", true)));
        assert!(!r.take([], t1 + INSERTED_FOR - ms(1)));
        assert!(r.take([], t1 + INSERTED_FOR), "it goes: redraw");
        assert!(r.shown.is_none());
        assert!(!r.take([], t1 + INSERTED_FOR + ms(16)));
    }

    #[test]
    fn an_unknown_app_is_just_inserted() {
        let mut r = Rail::default();
        r.take([VoiceEvent::Delivered { id: 1, app: String::new() }], Instant::now());
        assert_eq!(note(&r), Some(("Inserted", true)));
    }

    #[test]
    fn a_failure_shows_its_reason() {
        let t0 = Instant::now();
        let mut r = Rail::default();
        r.take([VoiceEvent::Failed { id: 1, error: "Didn't catch that".into() }], t0);
        assert_eq!(note(&r), Some(("Didn't catch that", false)));
        assert!(r.take([], t0 + FAILED_FOR));
        assert!(r.shown.is_none());
    }

    #[test]
    fn a_tap_or_a_new_hold_keeps_the_last_result_until_listening_shows() {
        let t0 = Instant::now();
        let mut r = Rail::default();
        r.take([VoiceEvent::Delivered { id: 1, app: "Notepad".into() }], t0);
        // A Right Ctrl shortcut straight after: the result stays.
        r.take([started(2), VoiceEvent::Cancelled { id: 2 }], t0 + ms(200));
        assert_eq!(note(&r), Some(("Inserted into Notepad", true)));
        // A new hold: still the result during the tap window, then Listening.
        r.take([started(3)], t0 + ms(400));
        assert_eq!(note(&r), Some(("Inserted into Notepad", true)));
        r.take([], t0 + ms(400) + TAP);
        assert!(matches!(r.shown, Some(View::Listening { .. })));
        // Cancelled now: the old result doesn't come back.
        r.take([VoiceEvent::Cancelled { id: 3 }], t0 + ms(800));
        assert!(r.shown.is_none());
    }

    #[test]
    fn a_session_that_goes_quiet_stops_showing() {
        let t0 = Instant::now();
        let mut r = Rail::default();
        r.take([VoiceEvent::Transcribing { id: 1 }], t0);
        assert!(!r.take([], t0 + TRANSCRIBING_LIMIT));
        assert!(r.take([], t0 + TRANSCRIBING_LIMIT + ms(1)));
        assert!(r.shown.is_none());

        r.take([started(2)], t0);
        r.take([], t0 + LISTENING_LIMIT + ms(1));
        assert!(r.shown.is_none());
    }
}
