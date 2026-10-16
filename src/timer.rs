//! A countdown timer: set a length, then start, pause, resume or reset it. When it reaches
//! zero it stops and waits, ready to run again. Time is always passed in, which keeps this
//! easy to test.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    Idle,
    Running { ends_at: Instant },
    Paused { remaining: Duration },
}

#[derive(Debug)]
pub struct Timer {
    length: Duration,
    run: Run,
}

impl Timer {
    pub fn new(length: Duration) -> Self {
        Self { length, run: Run::Idle }
    }

    pub fn length(&self) -> Duration {
        self.length
    }

    /// Takes effect the next time the timer starts from the beginning.
    pub fn set_length(&mut self, length: Duration) {
        self.length = length;
    }

    pub fn is_running(&self) -> bool {
        matches!(self.run, Run::Running { .. })
    }

    pub fn is_idle(&self) -> bool {
        self.run == Run::Idle
    }

    pub fn remaining(&self, now: Instant) -> Duration {
        match self.run {
            Run::Idle => self.length,
            Run::Running { ends_at } => ends_at.saturating_duration_since(now),
            Run::Paused { remaining } => remaining,
        }
    }

    /// Start, pause or resume.
    pub fn toggle(&mut self, now: Instant) {
        self.run = match self.run {
            Run::Idle => Run::Running { ends_at: now + self.length },
            Run::Running { ends_at } => Run::Paused { remaining: ends_at.saturating_duration_since(now) },
            Run::Paused { remaining } => Run::Running { ends_at: now + remaining },
        };
    }

    /// Back to the full length, stopped.
    pub fn reset(&mut self) {
        self.run = Run::Idle;
    }

    /// Advances time. Returns true once, when the timer reaches zero (it's then idle again).
    pub fn tick(&mut self, now: Instant) -> bool {
        match self.run {
            Run::Running { ends_at } if now >= ends_at => {
                self.run = Run::Idle;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn runs_down_then_waits_ready_again() {
        let t0 = Instant::now();
        let mut t = Timer::new(25 * MIN);
        assert!(t.is_idle());
        assert_eq!(t.remaining(t0), 25 * MIN);
        t.toggle(t0);
        assert!(t.is_running());
        assert_eq!(t.remaining(t0 + 10 * MIN), 15 * MIN);
        assert!(!t.tick(t0 + 24 * MIN));
        assert!(t.tick(t0 + 25 * MIN), "finishes once");
        assert!(t.is_idle());
        assert!(!t.tick(t0 + 26 * MIN));
        assert_eq!(t.remaining(t0 + 30 * MIN), 25 * MIN, "ready to run again");
    }

    #[test]
    fn pause_holds_the_remaining_time() {
        let t0 = Instant::now();
        let mut t = Timer::new(25 * MIN);
        t.toggle(t0);
        t.toggle(t0 + 5 * MIN); // pause
        assert!(!t.is_running() && !t.is_idle());
        assert_eq!(t.remaining(t0 + 60 * MIN), 20 * MIN);
        assert!(!t.tick(t0 + 60 * MIN));
        t.toggle(t0 + 60 * MIN); // resume
        assert!(!t.tick(t0 + 79 * MIN));
        assert!(t.tick(t0 + 80 * MIN));
    }

    #[test]
    fn reset_and_length() {
        let t0 = Instant::now();
        let mut t = Timer::new(25 * MIN);
        t.toggle(t0);
        t.reset();
        assert!(t.is_idle());
        t.set_length(50 * MIN);
        assert_eq!(t.remaining(t0 + MIN), 50 * MIN);
    }
}
