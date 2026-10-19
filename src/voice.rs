//! The notch's side of MyNotch Voice (the `mynotch-voice` crate does the work): what the
//! collapsed rail says while you dictate. "● Listening" while Right Ctrl is held (once the
//! microphone is open and the hold is longer than a tap), "Transcribing…", then "✓ Inserted"
//! for a moment, or a few words on what went wrong. It never opens the notch or takes the focus.

use std::time::{Duration, Instant};

use mynotch_voice::audio::MAX_SECONDS;
use mynotch_voice::{Config, TAP, VoiceEvent, VoiceHandle, VoiceService};

use crate::widgets::Compact;

/// Above every widget's readout (the highest, a running timer, is 20).
const PRIORITY: u8 = 30;
const INSERTED_FOR: Duration = Duration::from_millis(1500);
const FAILED_FOR: Duration = Duration::from_secs(3);
/// A session that's been quiet this long ended without saying so (voice stopped, say): stop
/// showing it rather than hide the widgets' readouts for good.
const LISTENING_LIMIT: Duration = Duration::from_secs(MAX_SECONDS as u64 + 10);
const TRANSCRIBING_LIMIT: Duration = Duration::from_secs(60);

pub struct Voice {
    handle: VoiceHandle,
    rail: Rail,
}

impl Voice {
    pub fn start() -> Self {
        Self { handle: VoiceService::start(Config::from_env()), rail: Rail::default() }
    }

    /// Takes in what voice reported. Returns whether the rail needs a redraw.
    pub fn poll(&mut self, now: Instant) -> bool {
        self.rail.take(self.handle.events(), now)
    }

    pub fn compact(&self) -> Option<Compact> {
        self.rail.compact()
    }

    /// The live dot breathes while listening.
    pub fn animating(&self) -> bool {
        self.rail.animating()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Session {
    #[default]
    Idle,
    Listening {
        since: Instant,
    },
    Transcribing {
        since: Instant,
    },
}

/// What the rail shows, worked out from voice's events (kept apart from the service so it's
/// unit-tested).
#[derive(Debug, Default)]
struct Rail {
    session: Session,
    /// The last result ("✓ Inserted", or why it failed) and when it stops showing. A new
    /// session's tap window, or a tap, leaves it up.
    note: Option<(String, Instant)>,
    /// What showed at the last update (text, live), so a redraw is asked for only on a change.
    shown: Option<(String, bool)>,
}

impl Rail {
    fn take(&mut self, events: impl IntoIterator<Item = VoiceEvent>, now: Instant) -> bool {
        for event in events {
            match event {
                VoiceEvent::SessionStarted { .. } => self.session = Session::Listening { since: now },
                VoiceEvent::Transcribing { .. } => self.session = Session::Transcribing { since: now },
                VoiceEvent::Transcript { .. } => {}
                VoiceEvent::Delivered { .. } => {
                    self.session = Session::Idle;
                    self.note = Some(("✓ Inserted".into(), now + INSERTED_FOR));
                }
                VoiceEvent::Cancelled { .. } => self.session = Session::Idle,
                VoiceEvent::Failed { error, .. } => {
                    self.session = Session::Idle;
                    self.note = Some((error, now + FAILED_FOR));
                }
            }
        }
        match self.session {
            Session::Listening { since } if now - since > LISTENING_LIMIT => self.session = Session::Idle,
            Session::Transcribing { since } if now - since > TRANSCRIBING_LIMIT => self.session = Session::Idle,
            _ => {}
        }
        if self.note.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.note = None;
        }
        let shown = match self.session {
            Session::Listening { since } if now - since >= TAP => {
                self.note = None;
                Some(("Listening".to_owned(), true))
            }
            Session::Transcribing { .. } => {
                self.note = None;
                Some(("Transcribing…".to_owned(), false))
            }
            _ => self.note.as_ref().map(|(text, _)| (text.clone(), false)),
        };
        let changed = shown != self.shown;
        self.shown = shown;
        changed
    }

    fn compact(&self) -> Option<Compact> {
        let (text, live) = self.shown.clone()?;
        Some(Compact { priority: PRIORITY, text, live, progress: None })
    }

    fn animating(&self) -> bool {
        self.shown.as_ref().is_some_and(|(_, live)| *live)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn text(r: &Rail) -> Option<&str> {
        r.shown.as_ref().map(|(t, _)| t.as_str())
    }

    fn started() -> VoiceEvent {
        VoiceEvent::SessionStarted { id: 1 }
    }

    #[test]
    fn listening_waits_out_a_tap_then_breathes() {
        let t0 = Instant::now();
        let mut r = Rail::default();
        assert!(!r.take([started()], t0), "nothing to show yet");
        assert!(!r.take([], t0 + TAP - ms(1)));
        assert!(r.compact().is_none());
        assert!(r.take([], t0 + TAP), "Listening appears: redraw");
        assert_eq!(text(&r), Some("Listening"));
        assert!(r.animating());
        let c = r.compact().unwrap();
        assert!(c.live && c.priority > 20);
        assert!(!r.take([], t0 + TAP + ms(16)), "then no more redraws are asked for");
    }

    #[test]
    fn a_tap_never_shows() {
        let t0 = Instant::now();
        let mut r = Rail::default();
        r.take([started()], t0);
        assert!(!r.take([VoiceEvent::Cancelled { id: 1 }], t0 + ms(100)));
        assert!(!r.take([], t0 + ms(1000)));
        assert!(r.compact().is_none());
    }

    #[test]
    fn inserted_shows_for_a_moment() {
        let t0 = Instant::now();
        let mut r = Rail::default();
        r.take([started()], t0);
        r.take([VoiceEvent::Transcribing { id: 1 }], t0 + ms(2000));
        assert_eq!(text(&r), Some("Transcribing…"));
        assert!(!r.animating(), "no live dot while transcribing");
        let t1 = t0 + ms(2300);
        assert!(r.take([VoiceEvent::Transcript { id: 1, text: "Hi".into() }, VoiceEvent::Delivered { id: 1 }], t1));
        assert_eq!(text(&r), Some("✓ Inserted"));
        assert!(!r.take([], t1 + INSERTED_FOR - ms(1)));
        assert!(r.take([], t1 + INSERTED_FOR), "it goes: redraw");
        assert!(r.compact().is_none());
        assert!(!r.take([], t1 + INSERTED_FOR + ms(16)));
    }

    #[test]
    fn a_failure_shows_its_reason() {
        let t0 = Instant::now();
        let mut r = Rail::default();
        r.take([VoiceEvent::Failed { id: 1, error: "Didn't catch that".into() }], t0);
        assert_eq!(text(&r), Some("Didn't catch that"));
        assert!(!r.compact().unwrap().live);
        assert!(r.take([], t0 + FAILED_FOR));
        assert!(r.compact().is_none());
    }

    #[test]
    fn a_tap_or_a_new_hold_keeps_the_last_result_until_listening_shows() {
        let t0 = Instant::now();
        let mut r = Rail::default();
        r.take([VoiceEvent::Delivered { id: 1 }], t0);
        // A Right Ctrl shortcut straight after: the result stays.
        r.take([VoiceEvent::SessionStarted { id: 2 }, VoiceEvent::Cancelled { id: 2 }], t0 + ms(200));
        assert_eq!(text(&r), Some("✓ Inserted"));
        // A new hold: still the result during the tap window, then Listening.
        r.take([VoiceEvent::SessionStarted { id: 3 }], t0 + ms(400));
        assert_eq!(text(&r), Some("✓ Inserted"));
        r.take([], t0 + ms(400) + TAP);
        assert_eq!(text(&r), Some("Listening"));
        // Cancelled now: the old result doesn't come back.
        r.take([VoiceEvent::Cancelled { id: 3 }], t0 + ms(800));
        assert!(r.compact().is_none());
    }

    #[test]
    fn a_session_that_goes_quiet_stops_showing() {
        let t0 = Instant::now();
        let mut r = Rail::default();
        r.take([VoiceEvent::Transcribing { id: 1 }], t0);
        assert!(!r.take([], t0 + TRANSCRIBING_LIMIT));
        assert!(r.take([], t0 + TRANSCRIBING_LIMIT + ms(1)));
        assert!(r.compact().is_none());

        r.take([started()], t0);
        r.take([], t0 + LISTENING_LIMIT + ms(1));
        assert!(r.compact().is_none());
    }
}
