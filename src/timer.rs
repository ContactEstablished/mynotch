//! Pomodoro state machine: focus sessions separated by short breaks, with a long break after
//! every `SESSIONS_PER_CYCLE` focus sessions. When a phase ends the timer waits, idle, at the
//! start of the next one. Time is always passed in, which keeps this easy to test.

use std::time::{Duration, Instant};

pub const SESSIONS_PER_CYCLE: u32 = 4;
const SHORT_BREAK: Duration = Duration::from_secs(5 * 60);
const LONG_BREAK: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    Idle,
    Running { ends_at: Instant },
    Paused { remaining: Duration },
}

#[derive(Debug)]
pub struct Timer {
    phase: Phase,
    focus: Duration,
    /// Focus sessions completed in the current cycle.
    completed: u32,
    run: Run,
}

impl Timer {
    pub fn new(focus: Duration) -> Self {
        Self { phase: Phase::Focus, focus, completed: 0, run: Run::Idle }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn completed(&self) -> u32 {
        self.completed
    }

    pub fn focus_len(&self) -> Duration {
        self.focus
    }

    /// Takes effect for the next focus session, or immediately if one is waiting to start.
    pub fn set_focus_len(&mut self, focus: Duration) {
        self.focus = focus;
    }

    pub fn phase_len(&self) -> Duration {
        match self.phase {
            Phase::Focus => self.focus,
            Phase::ShortBreak => SHORT_BREAK,
            Phase::LongBreak => LONG_BREAK,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.run, Run::Running { .. })
    }

    pub fn is_idle(&self) -> bool {
        self.run == Run::Idle
    }

    pub fn remaining(&self, now: Instant) -> Duration {
        match self.run {
            Run::Idle => self.phase_len(),
            Run::Running { ends_at } => ends_at.saturating_duration_since(now),
            Run::Paused { remaining } => remaining,
        }
    }

    /// Start, pause or resume.
    pub fn toggle(&mut self, now: Instant) {
        self.run = match self.run {
            Run::Idle => Run::Running { ends_at: now + self.phase_len() },
            Run::Running { ends_at } => Run::Paused { remaining: ends_at.saturating_duration_since(now) },
            Run::Paused { remaining } => Run::Running { ends_at: now + remaining },
        };
    }

    /// Back to the start of the current phase.
    pub fn reset(&mut self) {
        self.run = Run::Idle;
    }

    /// Abandons the current phase and waits at the start of the next. A skipped focus
    /// session doesn't count towards the long break.
    pub fn skip(&mut self) {
        self.advance(false);
    }

    /// Advances time. Returns the phase that just finished, if any.
    pub fn tick(&mut self, now: Instant) -> Option<Phase> {
        match self.run {
            Run::Running { ends_at } if now >= ends_at => {
                let finished = self.phase;
                self.advance(true);
                Some(finished)
            }
            _ => None,
        }
    }

    fn advance(&mut self, finished: bool) {
        self.phase = match self.phase {
            Phase::Focus => {
                if finished {
                    self.completed += 1;
                }
                if self.completed >= SESSIONS_PER_CYCLE { Phase::LongBreak } else { Phase::ShortBreak }
            }
            Phase::ShortBreak => Phase::Focus,
            Phase::LongBreak => {
                self.completed = 0;
                Phase::Focus
            }
        };
        self.run = Run::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn focus_runs_then_waits_at_a_short_break() {
        let t0 = Instant::now();
        let mut t = Timer::new(25 * MIN);
        assert!(t.is_idle());
        assert_eq!(t.remaining(t0), 25 * MIN);

        t.toggle(t0);
        assert!(t.is_running());
        assert_eq!(t.remaining(t0 + 10 * MIN), 15 * MIN);
        assert_eq!(t.tick(t0 + 24 * MIN), None);
        assert_eq!(t.tick(t0 + 25 * MIN), Some(Phase::Focus));

        assert_eq!(t.phase(), Phase::ShortBreak);
        assert!(t.is_idle());
        assert_eq!(t.completed(), 1);
        assert_eq!(t.remaining(t0 + 30 * MIN), 5 * MIN);
    }

    #[test]
    fn pause_holds_the_remaining_time() {
        let t0 = Instant::now();
        let mut t = Timer::new(25 * MIN);
        t.toggle(t0);
        t.toggle(t0 + 5 * MIN); // pause
        assert!(!t.is_running() && !t.is_idle());
        assert_eq!(t.remaining(t0 + 60 * MIN), 20 * MIN);
        assert_eq!(t.tick(t0 + 60 * MIN), None);
        t.toggle(t0 + 60 * MIN); // resume
        assert_eq!(t.tick(t0 + 79 * MIN), None);
        assert_eq!(t.tick(t0 + 80 * MIN), Some(Phase::Focus));
    }

    #[test]
    fn long_break_after_four_sessions_then_cycle_restarts() {
        let mut now = Instant::now();
        let mut t = Timer::new(25 * MIN);
        let mut run_phase = |t: &mut Timer| {
            t.toggle(now);
            now += t.phase_len();
            t.tick(now)
        };
        for session in 1..=SESSIONS_PER_CYCLE {
            assert_eq!(run_phase(&mut t), Some(Phase::Focus));
            assert_eq!(t.completed(), session);
            let expected = if session < SESSIONS_PER_CYCLE { Phase::ShortBreak } else { Phase::LongBreak };
            assert_eq!(t.phase(), expected);
            if session < SESSIONS_PER_CYCLE {
                assert_eq!(run_phase(&mut t), Some(Phase::ShortBreak));
            }
        }
        assert_eq!(t.phase_len(), 15 * MIN);
        assert_eq!(run_phase(&mut t), Some(Phase::LongBreak));
        assert_eq!((t.phase(), t.completed()), (Phase::Focus, 0));
    }

    #[test]
    fn skipped_focus_does_not_count() {
        let mut t = Timer::new(25 * MIN);
        t.toggle(Instant::now());
        t.skip();
        assert_eq!((t.phase(), t.completed()), (Phase::ShortBreak, 0));
        assert!(t.is_idle());
        t.skip();
        assert_eq!(t.phase(), Phase::Focus);
    }

    #[test]
    fn reset_and_focus_length() {
        let t0 = Instant::now();
        let mut t = Timer::new(25 * MIN);
        t.toggle(t0);
        t.reset();
        assert!(t.is_idle());
        t.set_focus_len(50 * MIN);
        assert_eq!(t.remaining(t0 + MIN), 50 * MIN);
    }
}
