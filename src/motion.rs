//! Short transitions (hover, state changes) and the user's reduced-motion setting.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static REDUCED: AtomicBool = AtomicBool::new(false);

/// Whether the user turned Windows' animation effects off; transitions then jump straight
/// to their end and nothing pulses.
pub fn reduced() -> bool {
    REDUCED.load(Ordering::Relaxed)
}

pub fn set_reduced(on: bool) {
    REDUCED.store(on, Ordering::Relaxed);
}

/// Seconds since the first call: a shared clock for looping animations.
pub fn seconds() -> f32 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

/// How far through its loop an animation of `period` seconds is, 0..1.
pub fn cycle(period: f32) -> f32 {
    (seconds() / period).fract()
}

/// A gentle 0..1 rise and fall every `period` seconds.
pub fn breath(period: f32) -> f32 {
    0.5 - 0.5 * (cycle(period) * std::f32::consts::TAU).cos()
}

/// A stable key for a transition, from anything hashable (e.g. a control's rect).
pub fn key(parts: impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

/// The longest step one frame may take, so a frame after a long idle gap still animates.
const MAX_STEP: f32 = 0.05;

/// Values that ease towards their targets across frames, keyed by whatever asks for them.
/// A value nobody asks for during a frame is forgotten.
#[derive(Default)]
pub struct Motion {
    /// Value, and whether it was asked for (and stepped) this frame.
    values: RefCell<HashMap<u64, (f32, bool)>>,
    /// Seconds since the previous frame, capped at `MAX_STEP`.
    dt: Cell<f32>,
    last: Cell<Option<Instant>>,
    moving: Cell<bool>,
}

impl Motion {
    pub fn begin(&self, now: Instant) {
        let dt = self.last.get().map_or(0.0, |last| now.saturating_duration_since(last).as_secs_f32());
        self.dt.set(dt.min(MAX_STEP));
        self.last.set(Some(now));
        self.moving.set(false);
        for v in self.values.borrow_mut().values_mut() {
            v.1 = false;
        }
    }

    /// Ends the frame. Returns whether anything is still on its way, i.e. whether to draw
    /// another frame.
    pub fn end(&self) -> bool {
        self.values.borrow_mut().retain(|_, v| v.1);
        self.moving.get()
    }

    /// `key`'s value this frame, eased towards `target` so that it gets (almost) there in
    /// `over`. A new key starts at its target. Asking twice in a frame doesn't step twice.
    pub fn ease(&self, key: u64, target: f32, over: Duration) -> f32 {
        let mut values = self.values.borrow_mut();
        let (value, stepped) = values.entry(key).or_insert((target, false));
        if !*stepped {
            *stepped = true;
            *value = step(*value, target, self.dt.get(), over, reduced());
        }
        if *value != target {
            self.moving.set(true);
        }
        *value
    }
}

/// Exponential approach: about 95% of the way in `over`, then a snap to the target; with
/// animations `reduced`, straight there.
fn step(value: f32, target: f32, dt: f32, over: Duration, reduced: bool) -> f32 {
    if reduced || over.is_zero() {
        return target;
    }
    let next = value + (target - value) * (1.0 - (-3.0 * dt / over.as_secs_f32()).exp());
    if (target - next).abs() < 0.01 { target } else { next }
}

#[cfg(test)]
mod tests {
    use super::{Motion, key, step};
    use std::time::{Duration, Instant};

    #[test]
    fn eases_most_of_the_way_in_its_duration_then_snaps() {
        let over = Duration::from_millis(160);
        let mut v = 0.0;
        for _ in 0..10 {
            v = step(v, 1.0, 0.016, over, false);
        }
        assert!(v > 0.9 && v < 1.0, "{v}");
        for _ in 0..10 {
            v = step(v, 1.0, 0.016, over, false);
        }
        assert_eq!(v, 1.0);
        assert_eq!(step(0.0, 1.0, 0.016, over, true), 1.0, "reduced motion: no transition");
    }

    #[test]
    fn keys_are_stepped_once_a_frame_and_forgotten_when_unused() {
        let m = Motion::default();
        let t0 = Instant::now();
        let k = key(("hover", 1, 2));
        m.begin(t0);
        assert_eq!(m.ease(k, 0.0, Duration::from_millis(160)), 0.0, "starts at its target");
        assert!(!m.end());
        m.begin(t0 + Duration::from_millis(16));
        let a = m.ease(k, 1.0, Duration::from_millis(160));
        assert_eq!(m.ease(k, 1.0, Duration::from_millis(160)), a, "no second step");
        assert!(a > 0.0 && a < 1.0);
        assert!(m.end(), "still moving");
        m.begin(t0 + Duration::from_millis(32));
        assert!(!m.end());
        m.begin(t0 + Duration::from_millis(48));
        assert_eq!(m.ease(k, 1.0, Duration::from_millis(160)), 1.0, "forgotten, so it starts afresh");
    }
}
