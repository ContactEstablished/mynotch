//! The session state machine (plan §5.5, the part M0 needs), kept pure so its transitions are
//! unit-tested: it takes inputs and says what to do; `service` does it.
//!
//! Idle → Arming (opening the microphone) → Listening → Transcribing → Delivering → Idle (done:
//! `Delivered`). While the key is held (Arming, Listening) a tap or chord cancels; a failure at
//! any step ends the session (`Failed`). One session at a time: a new press while one is
//! transcribing or delivering is ignored.

use crate::VoiceEvent;

pub type SessionId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Arming(SessionId),
    Listening(SessionId),
    Transcribing(SessionId),
    Delivering(SessionId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// The trigger went down.
    Start,
    /// The trigger came up after a hold, or the session hit its length cap.
    Stop,
    /// A tap or a chord: drop the session.
    Cancel,
    MicReady(SessionId),
    MicFailed(SessionId, String),
    Transcribed(SessionId, String),
    TranscribeFailed(SessionId, String),
    Delivered(SessionId),
    DeliverFailed(SessionId, String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    OpenMic(SessionId),
    /// Stop capturing and throw the audio away.
    DropMic(SessionId),
    /// Stop capturing and transcribe what was heard.
    Transcribe(SessionId),
    Deliver(SessionId, String),
    Emit(VoiceEvent),
}

/// Shown when Whisper heard nothing it could write down.
pub const NOTHING_HEARD: &str = "Didn't catch that";
/// Shown when the key came up before the microphone had opened.
pub const MIC_NOT_READY: &str = "The mic wasn't ready";

#[derive(Debug)]
pub struct Pipeline {
    phase: Phase,
    last_id: SessionId,
    /// The microphone failed to open for the arming session. It's reported when the key comes
    /// up, so a tap or a chord (Right Ctrl+C) never shows it.
    mic_error: Option<String>,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl Pipeline {
    pub fn new() -> Self {
        Self { phase: Phase::Idle, last_id: 0, mic_error: None }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn handle(&mut self, input: Input) -> Vec<Effect> {
        use Effect::*;
        use Input::*;
        use Phase::*;
        let (phase, effects) = match (self.phase, input) {
            (Idle, Start) => self.start(vec![]),
            // A press while a session is held means its key-up was lost: drop it, start afresh.
            (Arming(id) | Listening(id), Start) => self.start(vec![DropMic(id), Emit(VoiceEvent::Cancelled { id })]),
            (Arming(id), Cancel) => (Idle, vec![Emit(VoiceEvent::Cancelled { id })]),
            // Held long enough to speak, but the microphone never opened.
            (Arming(id), Stop) => {
                let error = self.mic_error.take().unwrap_or_else(|| MIC_NOT_READY.into());
                (Idle, vec![Emit(VoiceEvent::Failed { id, error })])
            }
            (Arming(id), MicReady(m)) if m == id => (Listening(id), vec![Emit(VoiceEvent::SessionStarted { id })]),
            (Arming(id), MicFailed(m, error)) if m == id => {
                self.mic_error = Some(error);
                (Arming(id), vec![])
            }
            (Listening(id), Stop) => {
                (Transcribing(id), vec![Transcribe(id), Emit(VoiceEvent::Transcribing { id })])
            }
            (Listening(id), Cancel) => (Idle, vec![DropMic(id), Emit(VoiceEvent::Cancelled { id })]),
            (Transcribing(id), Transcribed(t, text)) if t == id => {
                if text.is_empty() {
                    (Idle, vec![Emit(VoiceEvent::Failed { id, error: NOTHING_HEARD.into() })])
                } else {
                    (Delivering(id), vec![Emit(VoiceEvent::Transcript { id, text: text.clone() }), Deliver(id, text)])
                }
            }
            (Transcribing(id), TranscribeFailed(t, error)) if t == id => (Idle, vec![Emit(VoiceEvent::Failed { id, error })]),
            (Delivering(id), Delivered(d)) if d == id => (Idle, vec![Emit(VoiceEvent::Delivered { id })]),
            (Delivering(id), DeliverFailed(d, error)) if d == id => (Idle, vec![Emit(VoiceEvent::Failed { id, error })]),
            // A microphone that finished opening for a session that's gone.
            (phase, MicReady(stale)) => (phase, vec![DropMic(stale)]),
            // Anything else is ignored: presses while busy, stray releases, late results.
            (phase, _) => (phase, vec![]),
        };
        if !matches!(phase, Arming(_)) {
            self.mic_error = None;
        }
        self.phase = phase;
        effects
    }

    /// A new session, after `before` (dropping an old one).
    fn start(&mut self, mut before: Vec<Effect>) -> (Phase, Vec<Effect>) {
        self.last_id += 1;
        self.mic_error = None;
        before.push(Effect::OpenMic(self.last_id));
        (Phase::Arming(self.last_id), before)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Effect::*;

    fn emit(e: VoiceEvent) -> Effect {
        Emit(e)
    }

    #[test]
    fn a_full_session_runs_from_press_to_paste() {
        let mut p = Pipeline::new();
        assert_eq!(p.handle(Input::Start), vec![OpenMic(1)]);
        assert_eq!(p.phase(), Phase::Arming(1));
        assert_eq!(p.handle(Input::MicReady(1)), vec![emit(VoiceEvent::SessionStarted { id: 1 })]);
        assert_eq!(p.handle(Input::Stop), vec![Transcribe(1), emit(VoiceEvent::Transcribing { id: 1 })]);
        assert_eq!(
            p.handle(Input::Transcribed(1, "Hello.".into())),
            vec![emit(VoiceEvent::Transcript { id: 1, text: "Hello.".into() }), Deliver(1, "Hello.".into())]
        );
        assert_eq!(p.phase(), Phase::Delivering(1));
        assert_eq!(p.handle(Input::Delivered(1)), vec![emit(VoiceEvent::Delivered { id: 1 })]);
        assert_eq!(p.phase(), Phase::Idle);
        assert_eq!(p.handle(Input::Start), vec![OpenMic(2)], "the next session gets a new id");
    }

    #[test]
    fn cancel_while_listening_drops_the_audio() {
        let mut p = Pipeline::new();
        p.handle(Input::Start);
        p.handle(Input::MicReady(1));
        assert_eq!(p.handle(Input::Cancel), vec![DropMic(1), emit(VoiceEvent::Cancelled { id: 1 })]);
        assert_eq!(p.phase(), Phase::Idle);
    }

    #[test]
    fn a_tap_before_the_mic_opens_cancels_and_the_late_mic_is_closed() {
        let mut p = Pipeline::new();
        p.handle(Input::Start);
        assert_eq!(p.handle(Input::Cancel), vec![emit(VoiceEvent::Cancelled { id: 1 })]);
        assert_eq!(p.handle(Input::MicReady(1)), vec![DropMic(1)]);
        assert_eq!(p.phase(), Phase::Idle);
    }

    #[test]
    fn a_hold_released_before_the_mic_opens_says_so() {
        let mut p = Pipeline::new();
        p.handle(Input::Start);
        assert_eq!(p.handle(Input::Stop), vec![emit(VoiceEvent::Failed { id: 1, error: MIC_NOT_READY.into() })]);
        assert_eq!(p.handle(Input::MicReady(1)), vec![DropMic(1)]);
        assert_eq!(p.phase(), Phase::Idle);
    }

    #[test]
    fn a_failing_mic_is_reported_only_for_a_hold() {
        // A tap or chord (Right Ctrl+C) with no microphone shows nothing.
        let mut p = Pipeline::new();
        p.handle(Input::Start);
        assert_eq!(p.handle(Input::MicFailed(1, "No microphone found".into())), vec![]);
        assert_eq!(p.handle(Input::Cancel), vec![emit(VoiceEvent::Cancelled { id: 1 })]);
        // A hold does.
        p.handle(Input::Start);
        p.handle(Input::MicFailed(2, "No microphone found".into()));
        assert_eq!(p.handle(Input::Stop), vec![emit(VoiceEvent::Failed { id: 2, error: "No microphone found".into() })]);
        assert_eq!(p.phase(), Phase::Idle);
        // And the error doesn't carry over to the next session.
        p.handle(Input::Start);
        assert_eq!(p.handle(Input::Stop), vec![emit(VoiceEvent::Failed { id: 3, error: MIC_NOT_READY.into() })]);
    }

    #[test]
    fn a_press_while_held_drops_the_session_whose_key_up_was_lost() {
        let mut p = Pipeline::new();
        p.handle(Input::Start);
        p.handle(Input::MicReady(1));
        assert_eq!(p.handle(Input::Start), vec![DropMic(1), emit(VoiceEvent::Cancelled { id: 1 }), OpenMic(2)]);
        assert_eq!(p.phase(), Phase::Arming(2));
        // Also while the microphone is still opening; its late arrival is closed.
        assert_eq!(p.handle(Input::Start), vec![DropMic(2), emit(VoiceEvent::Cancelled { id: 2 }), OpenMic(3)]);
        assert_eq!(p.handle(Input::MicReady(2)), vec![DropMic(2)]);
        assert_eq!(p.handle(Input::MicReady(3)), vec![emit(VoiceEvent::SessionStarted { id: 3 })]);
    }

    #[test]
    fn stale_mic_news_doesnt_disturb_a_newer_session() {
        let mut p = Pipeline::new();
        p.handle(Input::Start);
        p.handle(Input::Cancel);
        p.handle(Input::Start);
        assert_eq!(p.handle(Input::MicFailed(1, "busy".into())), vec![]);
        assert_eq!(p.handle(Input::MicReady(2)), vec![emit(VoiceEvent::SessionStarted { id: 2 })]);
        assert_eq!(p.handle(Input::MicReady(1)), vec![DropMic(1)], "while listening");
        p.handle(Input::Stop);
        assert_eq!(p.handle(Input::MicReady(1)), vec![DropMic(1)], "while transcribing");
        assert_eq!(p.phase(), Phase::Transcribing(2));
    }

    #[test]
    fn a_late_mic_from_an_old_session_is_closed_without_disturbing_the_new_one() {
        let mut p = Pipeline::new();
        p.handle(Input::Start);
        p.handle(Input::Cancel);
        p.handle(Input::Start);
        assert_eq!(p.handle(Input::MicReady(1)), vec![DropMic(1)]);
        assert_eq!(p.phase(), Phase::Arming(2));
        assert_eq!(p.handle(Input::MicReady(2)), vec![emit(VoiceEvent::SessionStarted { id: 2 })]);
    }

    #[test]
    fn presses_while_transcribing_or_delivering_are_ignored() {
        let mut p = Pipeline::new();
        p.handle(Input::Start);
        p.handle(Input::MicReady(1));
        p.handle(Input::Stop);
        for input in [Input::Start, Input::Stop, Input::Cancel] {
            assert_eq!(p.handle(input), vec![]);
            assert_eq!(p.phase(), Phase::Transcribing(1));
        }
        p.handle(Input::Transcribed(1, "Hi".into()));
        for input in [Input::Start, Input::Stop, Input::Cancel] {
            assert_eq!(p.handle(input), vec![]);
            assert_eq!(p.phase(), Phase::Delivering(1));
        }
    }

    #[test]
    fn failures_end_the_session() {
        let mut p = Pipeline::new();
        p.handle(Input::Start);
        p.handle(Input::MicFailed(1, "No microphone found".into()));
        assert_eq!(
            p.handle(Input::Stop),
            vec![emit(VoiceEvent::Failed { id: 1, error: "No microphone found".into() })]
        );
        assert_eq!(p.phase(), Phase::Idle);

        p.handle(Input::Start);
        p.handle(Input::MicReady(2));
        p.handle(Input::Stop);
        assert_eq!(
            p.handle(Input::TranscribeFailed(2, "Whisper failed".into())),
            vec![emit(VoiceEvent::Failed { id: 2, error: "Whisper failed".into() })]
        );

        p.handle(Input::Start);
        p.handle(Input::MicReady(3));
        p.handle(Input::Stop);
        p.handle(Input::Transcribed(3, "Hi".into()));
        assert_eq!(
            p.handle(Input::DeliverFailed(3, "The clipboard is busy".into())),
            vec![emit(VoiceEvent::Failed { id: 3, error: "The clipboard is busy".into() })]
        );
        assert_eq!(p.phase(), Phase::Idle);
    }

    #[test]
    fn hearing_nothing_pastes_nothing() {
        let mut p = Pipeline::new();
        p.handle(Input::Start);
        p.handle(Input::MicReady(1));
        p.handle(Input::Stop);
        assert_eq!(
            p.handle(Input::Transcribed(1, String::new())),
            vec![emit(VoiceEvent::Failed { id: 1, error: NOTHING_HEARD.into() })]
        );
        assert_eq!(p.phase(), Phase::Idle);
    }

    #[test]
    fn results_for_other_sessions_are_ignored() {
        let mut p = Pipeline::new();
        p.handle(Input::Start);
        p.handle(Input::MicReady(1));
        p.handle(Input::Stop);
        assert_eq!(p.handle(Input::Transcribed(7, "Hi".into())), vec![]);
        assert_eq!(p.handle(Input::Delivered(1)), vec![], "not delivering yet");
        assert_eq!(p.phase(), Phase::Transcribing(1));
    }

    #[test]
    fn stray_inputs_while_idle_do_nothing() {
        let mut p = Pipeline::new();
        for input in [Input::Stop, Input::Cancel, Input::Delivered(1), Input::MicFailed(1, "x".into())] {
            assert_eq!(p.handle(input), vec![]);
            assert_eq!(p.phase(), Phase::Idle);
        }
    }
}
