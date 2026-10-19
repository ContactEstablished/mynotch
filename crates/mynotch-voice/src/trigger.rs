//! What the keyboard hook does with each key event: the push-to-talk rules as a pure state
//! machine, so they're unit-tested without a hook (`platform::win::hook` feeds it).
//!
//! - Right Ctrl down starts a hold; its down and up are swallowed, so the app under the cursor
//!   never sees a stray Ctrl. Auto-repeat downs while held are swallowed too.
//! - Let go within `TAP_MS` and it was a tap: the session is dropped, and the tap is replayed
//!   so apps that use Right Ctrl on its own still get it.
//! - Any other key going down while Right Ctrl is held is a chord (Right Ctrl+C, Right
//!   Ctrl+Alt+K): the session is cancelled, Right Ctrl's down is replayed ahead of that key, and
//!   everything after passes through until Right Ctrl comes up, so the app sees a normal chord.
//! - MyNotch's own injected input is never a trigger. Other injected input is ignored unless
//!   `accept_injected` (a test switch) is set.
//! - A key-up can go missing (the user let go over an elevated window, which a non-elevated
//!   hook never hears from). While held, the key auto-repeats; when it has been silent for
//!   `LOST_UP_MS`, the next key event treats it as already up.

/// `VK_RCONTROL`.
pub const TRIGGER_VK: u32 = 0xA3;
/// A hold shorter than this is a tap, not a session.
pub const TAP_MS: u32 = 150;
/// Auto-repeat downs arrive every few tens of ms once repeating starts, after at most about a
/// second. A held trigger silent for longer than this was let go without our hearing it.
/// (With Windows' key repeat turned off there are no repeats, and a chord pressed more than
/// this long after Right Ctrl loses its Ctrl.)
const LOST_UP_MS: u32 = 1500;

/// One low-level key event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub vk: u32,
    pub scan: u32,
    pub down: bool,
    /// `LLKHF_EXTENDED`: replayed keys keep it.
    pub extended: bool,
    /// `LLKHF_INJECTED`: synthesized by some program, not the keyboard.
    pub injected: bool,
    /// Injected by MyNotch itself (it carries MyNotch's marker).
    pub ours: bool,
    /// The event's timestamp in ms (`KBDLLHOOKSTRUCT::time`; wraps after 49 days).
    pub time: u32,
}

/// What the pipeline hears about the trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// Right Ctrl went down: start listening.
    Press,
    /// Right Ctrl came up after a hold: transcribe and paste.
    Release,
    /// Released too soon (a tap), another key made it a chord, or the key-up was lost: drop
    /// the session.
    Cancel,
}

/// Keys the hook injects in place of what it swallowed, tagged as MyNotch's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replay {
    /// Right Ctrl down, then a copy of this event, so the chord reaches the app in the order
    /// it was typed.
    Chord,
    /// Right Ctrl down and up: the tap, as the plain key press it was.
    Tap,
}

/// What the hook does with the event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Decision {
    /// Don't pass the event on.
    pub swallow: bool,
    pub replay: Option<Replay>,
    pub signal: Option<Signal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    /// Right Ctrl is down and swallowed; `last` is the latest trigger event's time.
    Held { since: u32, last: u32 },
    /// A chord: Right Ctrl was replayed, so everything passes until it comes up.
    Chord { last: u32 },
}

#[derive(Debug)]
pub struct Trigger {
    state: State,
    accept_injected: bool,
}

const PASS: Decision = Decision { swallow: false, replay: None, signal: None };
const SWALLOW: Decision = Decision { swallow: true, replay: None, signal: None };

impl Trigger {
    pub fn new(accept_injected: bool) -> Self {
        Self { state: State::Idle, accept_injected }
    }

    /// Whether a session is being held (Right Ctrl down and swallowed).
    pub fn held(&self) -> bool {
        matches!(self.state, State::Held { .. })
    }

    /// Forget any key in progress (after the hook was re-installed, when key-ups may have been
    /// missed). Returns `Cancel` if a hold was in progress.
    pub fn reset(&mut self) -> Option<Signal> {
        let was_held = self.held();
        self.state = State::Idle;
        was_held.then_some(Signal::Cancel)
    }

    fn press(&mut self, time: u32) -> Decision {
        self.state = State::Held { since: time, last: time };
        Decision { signal: Some(Signal::Press), ..SWALLOW }
    }

    pub fn on_key(&mut self, e: KeyEvent) -> Decision {
        if e.ours || (e.injected && !self.accept_injected) {
            return PASS;
        }
        let silent = |last: u32| e.time.wrapping_sub(last) > LOST_UP_MS;
        if e.vk == TRIGGER_VK {
            return match (self.state, e.down) {
                // A down after a silence is a new press: the last up was lost. (The pipeline
                // drops a session still held when it hears this new press.)
                (State::Idle, true) => self.press(e.time),
                (State::Held { last, .. } | State::Chord { last }, true) if silent(last) => self.press(e.time),
                (State::Held { since, .. }, true) => {
                    self.state = State::Held { since, last: e.time };
                    SWALLOW
                }
                (State::Held { since, .. }, false) => {
                    self.state = State::Idle;
                    if e.time.wrapping_sub(since) < TAP_MS {
                        Decision { swallow: true, replay: Some(Replay::Tap), signal: Some(Signal::Cancel) }
                    } else {
                        Decision { signal: Some(Signal::Release), ..SWALLOW }
                    }
                }
                // Repeats during a chord reach the app with the replayed down before them.
                (State::Chord { .. }, true) => {
                    self.state = State::Chord { last: e.time };
                    PASS
                }
                (State::Chord { .. }, false) => {
                    self.state = State::Idle;
                    PASS
                }
                // A stray up (the key was down before the hook was installed).
                (State::Idle, false) => PASS,
            };
        }
        match self.state {
            State::Held { last, .. } if e.down && silent(last) => {
                // Right Ctrl was let go unheard: this key is typed on its own.
                self.state = State::Idle;
                Decision { signal: Some(Signal::Cancel), ..PASS }
            }
            State::Held { .. } if e.down => {
                self.state = State::Chord { last: e.time };
                Decision { swallow: true, replay: Some(Replay::Chord), signal: Some(Signal::Cancel) }
            }
            State::Chord { last } if e.down && silent(last) => {
                // Right Ctrl's up was lost during the chord; nothing is held any more.
                self.state = State::Idle;
                PASS
            }
            _ => PASS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u32 = 0x41;
    const LALT: u32 = 0xA4;

    fn key(vk: u32, down: bool, time: u32) -> KeyEvent {
        KeyEvent { vk, scan: 0, down, extended: false, injected: false, ours: false, time }
    }

    fn rctrl(down: bool, time: u32) -> KeyEvent {
        KeyEvent { extended: true, ..key(TRIGGER_VK, down, time) }
    }

    fn swallowed(signal: Option<Signal>) -> Decision {
        Decision { swallow: true, replay: None, signal }
    }

    fn passed() -> Decision {
        Decision::default()
    }

    fn chord() -> Decision {
        Decision { swallow: true, replay: Some(Replay::Chord), signal: Some(Signal::Cancel) }
    }

    #[test]
    fn a_hold_presses_and_releases_with_both_keys_swallowed() {
        let mut t = Trigger::new(false);
        assert_eq!(t.on_key(rctrl(true, 1000)), swallowed(Some(Signal::Press)));
        assert!(t.held());
        assert_eq!(t.on_key(rctrl(false, 3000)), swallowed(Some(Signal::Release)));
        assert!(!t.held());
    }

    #[test]
    fn a_short_hold_is_a_tap_and_is_replayed() {
        let mut t = Trigger::new(false);
        t.on_key(rctrl(true, 1000));
        assert_eq!(
            t.on_key(rctrl(false, 1000 + TAP_MS - 1)),
            Decision { swallow: true, replay: Some(Replay::Tap), signal: Some(Signal::Cancel) }
        );
        t.on_key(rctrl(true, 5000));
        assert_eq!(t.on_key(rctrl(false, 5000 + TAP_MS)), swallowed(Some(Signal::Release)), "exactly TAP_MS is a hold");
    }

    #[test]
    fn a_tap_across_the_clock_wrap() {
        let mut t = Trigger::new(false);
        t.on_key(rctrl(true, u32::MAX - 40));
        assert_eq!(t.on_key(rctrl(false, 60)).signal, Some(Signal::Cancel), "101 ms, wrapped");
    }

    #[test]
    fn auto_repeat_while_held_is_swallowed_without_a_signal() {
        let mut t = Trigger::new(false);
        t.on_key(rctrl(true, 1000));
        for i in 1..40 {
            assert_eq!(t.on_key(rctrl(true, 1000 + 500 + i * 33)), swallowed(None));
        }
        assert_eq!(t.on_key(rctrl(false, 4000)), swallowed(Some(Signal::Release)));
    }

    #[test]
    fn a_chord_cancels_replays_ctrl_and_then_passes_everything() {
        let mut t = Trigger::new(false);
        t.on_key(rctrl(true, 1000));
        assert_eq!(t.on_key(key(A, true, 1080)), chord(), "A is swallowed and replayed after Right Ctrl");
        assert!(!t.held());
        assert_eq!(t.on_key(rctrl(true, 1120)), passed(), "repeats reach the app");
        assert_eq!(t.on_key(key(A, false, 1150)), passed());
        assert_eq!(t.on_key(rctrl(false, 1300)), passed(), "the up isn't swallowed: the app saw the down");
        // Back to normal.
        assert_eq!(t.on_key(rctrl(true, 2000)), swallowed(Some(Signal::Press)));
    }

    #[test]
    fn a_modifier_makes_a_chord_too() {
        // Right Ctrl+Alt+K: Alt is replayed, K then passes on its own.
        let mut t = Trigger::new(false);
        t.on_key(rctrl(true, 1000));
        assert_eq!(t.on_key(key(LALT, true, 1040)), chord());
        assert_eq!(t.on_key(key(0x4B, true, 1090)), passed());
    }

    #[test]
    fn a_chord_late_in_a_long_hold_still_replays() {
        let mut t = Trigger::new(false);
        t.on_key(rctrl(true, 1000));
        // Held for 10 s, repeating.
        for i in 0..300 {
            t.on_key(rctrl(true, 1500 + i * 33));
        }
        assert_eq!(t.on_key(key(A, true, 1500 + 300 * 33)), chord());
    }

    #[test]
    fn other_keys_pass_when_nothing_is_held() {
        let mut t = Trigger::new(false);
        assert_eq!(t.on_key(key(A, true, 1000)), passed());
        assert_eq!(t.on_key(key(A, false, 1050)), passed());
        // A key-up while held (it went down before Right Ctrl) isn't a chord.
        t.on_key(key(A, true, 1100));
        t.on_key(rctrl(true, 1200));
        assert_eq!(t.on_key(key(A, false, 1250)), passed());
        assert!(t.held());
    }

    #[test]
    fn a_stray_trigger_up_passes() {
        let mut t = Trigger::new(false);
        assert_eq!(t.on_key(rctrl(false, 1000)), passed());
    }

    #[test]
    fn our_own_injected_input_is_never_a_trigger() {
        for accept_injected in [false, true] {
            let mut t = Trigger::new(accept_injected);
            let ours = |e: KeyEvent| KeyEvent { injected: true, ours: true, ..e };
            assert_eq!(t.on_key(ours(rctrl(true, 1000))), passed());
            assert!(!t.held());
            // Nor does it make a chord: the paste's Ctrl+V while a hold is in progress.
            t.on_key(rctrl(true, 2000));
            assert_eq!(t.on_key(ours(key(0x56, true, 2100))), passed());
            assert!(t.held());
        }
    }

    #[test]
    fn foreign_injected_input_counts_only_when_accepted() {
        let injected = |e: KeyEvent| KeyEvent { injected: true, ..e };
        let mut t = Trigger::new(false);
        assert_eq!(t.on_key(injected(rctrl(true, 1000))), passed());
        assert!(!t.held());
        // Nor does another program's key make a chord of a real hold.
        t.on_key(rctrl(true, 2000));
        assert_eq!(t.on_key(injected(key(A, true, 2100))), passed());
        assert!(t.held());

        let mut t = Trigger::new(true);
        assert_eq!(t.on_key(injected(rctrl(true, 1000))), swallowed(Some(Signal::Press)));
        assert_eq!(t.on_key(injected(rctrl(false, 3000))), swallowed(Some(Signal::Release)));
    }

    #[test]
    fn a_lost_key_up_ends_at_the_next_press() {
        let mut t = Trigger::new(false);
        t.on_key(rctrl(true, 1000));
        assert_eq!(t.on_key(rctrl(true, 1000 + LOST_UP_MS)), swallowed(None), "exactly LOST_UP_MS: still a repeat");
        assert_eq!(t.on_key(rctrl(true, 1000 + 2 * LOST_UP_MS + 1)), swallowed(Some(Signal::Press)));
        assert_eq!(t.on_key(rctrl(false, 9000)), swallowed(Some(Signal::Release)));
    }

    #[test]
    fn a_lost_key_up_ends_at_the_next_key_without_a_phantom_ctrl() {
        let mut t = Trigger::new(false);
        t.on_key(rctrl(true, 1000));
        // Let go over an elevated window; later the user types an ordinary key.
        assert_eq!(t.on_key(key(A, true, 1000 + LOST_UP_MS + 1)), Decision { signal: Some(Signal::Cancel), ..passed() });
        assert!(!t.held());
        assert_eq!(t.on_key(key(A, true, 5000)), passed());
    }

    #[test]
    fn a_lost_key_up_during_a_chord_recovers() {
        let mut t = Trigger::new(false);
        t.on_key(rctrl(true, 1000));
        t.on_key(key(A, true, 1050));
        // Right Ctrl's up never arrives. The next press starts a session again...
        assert_eq!(t.on_key(rctrl(true, 1050 + LOST_UP_MS + 1)), swallowed(Some(Signal::Press)));
        // ...and a key long after a lost chord passes with nothing held.
        let mut t = Trigger::new(false);
        t.on_key(rctrl(true, 1000));
        t.on_key(key(A, true, 1050));
        assert_eq!(t.on_key(key(A, true, 1050 + LOST_UP_MS + 1)), passed());
        assert_eq!(t.on_key(rctrl(true, 9000)), swallowed(Some(Signal::Press)));
    }

    #[test]
    fn the_millisecond_clock_wrapping_is_harmless() {
        let mut t = Trigger::new(false);
        t.on_key(rctrl(true, u32::MAX - 50));
        assert_eq!(t.on_key(rctrl(true, 10)), swallowed(None), "a repeat across the wrap");
        assert_eq!(t.on_key(rctrl(false, 400)), swallowed(Some(Signal::Release)));
    }

    #[test]
    fn reset_cancels_a_hold() {
        let mut t = Trigger::new(false);
        assert_eq!(t.reset(), None);
        t.on_key(rctrl(true, 1000));
        assert_eq!(t.reset(), Some(Signal::Cancel));
        assert!(!t.held());
        assert_eq!(t.on_key(rctrl(true, 1100)), swallowed(Some(Signal::Press)));
    }
}
