//! Keep awake. While it's on, Windows neither sleeps nor turns the screen off, and after a
//! minute without input a tap of F15 (a key no app uses) keeps Teams and the like showing the
//! user as active. It always starts off, and ends by itself when its time is up, or below
//! `LOW_BATTERY` percent on battery. While the PC is locked it only holds it awake: there's
//! no one there to look active for. The Awake tab and the rail show it (`widgets::awake`).

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use windows::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, GetSystemPowerStatus, SYSTEM_POWER_STATUS,
    SetThreadExecutionState,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, LASTINPUTINFO,
    MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, VK_F15,
};

/// The slider's step, and the shortest length.
pub const STEP: u32 = 15;
/// The longest timed length; past it, the slider's last stop is no limit.
pub const MAX_MINUTES: u32 = 8 * 60;
/// Slider stops: every `STEP` up to `MAX_MINUTES`, then no limit.
const STOPS: u32 = MAX_MINUTES / STEP + 1;
/// How long without input before a tap of F15. Teams calls the user away after five minutes.
const IDLE: Duration = Duration::from_secs(60);
/// How often to look at the idle time.
const IDLE_POLL: Duration = Duration::from_secs(1);
/// How often to look at the battery.
const BATTERY_POLL: Duration = Duration::from_secs(30);
/// On battery below this percentage, Awake turns itself off so the PC can sleep.
pub const LOW_BATTERY: u8 = 20;
/// How long the rail says why Awake turned itself off.
const NOTE: Duration = Duration::from_secs(5);

/// How long Awake stays on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Length {
    /// Whole steps of `STEP`, `STEP..=MAX_MINUTES`.
    Minutes(u32),
    /// Until it's turned off.
    Forever,
}

impl Default for Length {
    fn default() -> Self {
        Self::Minutes(60)
    }
}

impl Length {
    /// The nearest step to `m` minutes, within the slider's range.
    pub fn minutes(m: u32) -> Self {
        let steps = (m.clamp(STEP, MAX_MINUTES) + STEP / 2) / STEP;
        Self::Minutes(steps * STEP)
    }

    /// The slider's position for this length, 0..=1.
    pub fn fraction(self) -> f32 {
        let stop = match self {
            Self::Minutes(m) => m.clamp(STEP, MAX_MINUTES) / STEP - 1,
            Self::Forever => STOPS - 1,
        };
        stop as f32 / (STOPS - 1) as f32
    }

    /// The length at slider position `k`, at the nearest stop.
    pub fn at(k: f32) -> Self {
        let stop = (k.clamp(0.0, 1.0) * (STOPS - 1) as f32).round() as u32;
        if stop >= STOPS - 1 { Self::Forever } else { Self::Minutes((stop + 1) * STEP) }
    }

    pub fn duration(self) -> Option<Duration> {
        match self {
            Self::Minutes(m) => Some(Duration::from_secs(u64::from(m) * 60)),
            Self::Forever => None,
        }
    }

    /// "45 m", "2 h", "2 h 15 m", or "No limit".
    pub fn label(self) -> String {
        match self {
            Self::Forever => "No limit".into(),
            Self::Minutes(m) if m < 60 => format!("{m} m"),
            Self::Minutes(m) if m % 60 == 0 => format!("{} h", m / 60),
            Self::Minutes(m) => format!("{} h {} m", m / 60, m % 60),
        }
    }

    /// How it's saved: minutes, or "forever".
    pub fn key(self) -> String {
        match self {
            Self::Minutes(m) => m.to_string(),
            Self::Forever => "forever".into(),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "forever" => Some(Self::Forever),
            _ => s.parse::<u32>().ok().map(Self::minutes),
        }
    }
}

/// Why Awake turned itself off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    TimeUp,
    LowBattery,
}

impl Ended {
    pub fn note(self) -> &'static str {
        match self {
            Self::TimeUp => "Awake ended",
            Self::LowBattery => "Awake off · battery low",
        }
    }
}

/// What the battery says, from `GetSystemPowerStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Power {
    plugged_in: bool,
    /// `None` without a battery, or when Windows doesn't know.
    percent: Option<u8>,
}

impl Power {
    fn low(self) -> bool {
        !self.plugged_in && self.percent.is_some_and(|p| p < LOW_BATTERY)
    }
}

/// Awake's state, without Windows: what's on, until when, and the note on why it last ended.
#[derive(Debug, Default)]
struct Core {
    length: Length,
    /// While on: when it ends, `None` for no limit.
    session: Option<Option<Instant>>,
    /// Why it last turned itself off, and when.
    ended: Option<(Ended, Instant)>,
}

impl Core {
    fn on(&self) -> bool {
        self.session.is_some()
    }

    fn start(&mut self, now: Instant) {
        self.session = Some(self.length.duration().map(|d| now + d));
        self.ended = None;
    }

    fn stop(&mut self) {
        self.session = None;
    }

    fn end(&mut self, why: Ended, now: Instant) {
        self.session = None;
        self.ended = Some((why, now));
    }

    /// A new length; while on, it counts from now.
    fn set_length(&mut self, length: Length, now: Instant) -> bool {
        let changed = length != self.length;
        self.length = length;
        if self.on() {
            self.start(now);
        }
        changed || self.on()
    }

    /// While on with a limit: what's left.
    fn remaining(&self, now: Instant) -> Option<Duration> {
        self.session.flatten().map(|end| end.saturating_duration_since(now))
    }

    /// Ends it once its time is up. Returns true when it did.
    fn tick(&mut self, now: Instant) -> bool {
        if self.session.flatten().is_some_and(|end| now >= end) {
            self.end(Ended::TimeUp, now);
            return true;
        }
        false
    }

    /// The note, while it's still showing.
    fn note(&self, now: Instant) -> Option<Ended> {
        self.ended.filter(|&(_, at)| now.saturating_duration_since(at) < NOTE).map(|(why, _)| why)
    }
}

/// Keep awake, shared by the shell (the rail and the menu) and the Awake tab. Lives on the UI
/// thread, which holds the PC awake (`SetThreadExecutionState` is per thread).
pub struct Awake {
    core: RefCell<Core>,
    /// What Windows was last told.
    held: Cell<bool>,
    /// The PC is locked: hold it awake, but don't tap keys.
    locked: Cell<bool>,
    /// The note was showing at the last tick, so its going gets drawn.
    noted: Cell<bool>,
    next_idle_check: Cell<Instant>,
    next_battery_check: Cell<Instant>,
}

impl Awake {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            core: RefCell::default(),
            held: Cell::new(false),
            locked: Cell::new(false),
            noted: Cell::new(false),
            next_idle_check: Cell::new(now),
            next_battery_check: Cell::new(now),
        }
    }

    pub fn on(&self) -> bool {
        self.core.borrow().on()
    }

    pub fn length(&self) -> Length {
        self.core.borrow().length
    }

    /// While on with a limit: what's left.
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.core.borrow().remaining(now)
    }

    /// Why it just turned itself off, for a few seconds after.
    pub fn note(&self, now: Instant) -> Option<Ended> {
        self.core.borrow().note(now)
    }

    /// Turns it on (for the chosen length) or off. Not on, below `LOW_BATTERY` on battery:
    /// the note says why.
    pub fn toggle(&self, now: Instant) {
        let mut core = self.core.borrow_mut();
        if core.on() {
            core.stop();
        } else if power().low() {
            core.end(Ended::LowBattery, now);
        } else {
            core.start(now);
            self.next_idle_check.set(now);
            self.next_battery_check.set(now + BATTERY_POLL);
        }
        drop(core);
        self.hold();
    }

    /// A new length; while on, it counts from now. Returns true when something shown changed.
    pub fn set_length(&self, length: Length, now: Instant) -> bool {
        self.core.borrow_mut().set_length(length, now)
    }

    /// The PC was locked or unlocked.
    pub fn set_locked(&self, locked: bool) {
        self.locked.set(locked);
        self.next_idle_check.set(Instant::now());
    }

    /// Per frame: ends it when its time is up or the battery runs low, and taps F15 after a
    /// minute without input. Returns true when something shown changed.
    pub fn tick(&self, now: Instant) -> bool {
        let mut changed = self.core.borrow_mut().tick(now);
        if self.on() && now >= self.next_battery_check.get() {
            self.next_battery_check.set(now + BATTERY_POLL);
            if power().low() {
                self.core.borrow_mut().end(Ended::LowBattery, now);
                changed = true;
            }
        }
        if changed {
            self.hold();
        }
        if self.on() && !self.locked.get() && now >= self.next_idle_check.get() {
            self.next_idle_check.set(now + IDLE_POLL);
            if idle() >= IDLE {
                tap_f15();
            }
        }
        let noted = self.note(now).is_some();
        changed | (self.noted.replace(noted) != noted)
    }

    /// Tells Windows whether to keep the PC and the screen awake, if that changed.
    fn hold(&self) {
        let on = self.on();
        if self.held.replace(on) != on {
            let flags = if on { ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED } else { ES_CONTINUOUS };
            unsafe { SetThreadExecutionState(flags) };
        }
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        self.core.get_mut().stop();
        self.hold();
    }
}

fn power() -> Power {
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
        return Power { plugged_in: true, percent: None };
    }
    // 128: no battery; 255: unknown.
    let battery = status.BatteryFlag != 128 && status.BatteryLifePercent <= 100;
    Power { plugged_in: status.ACLineStatus != 0, percent: battery.then_some(status.BatteryLifePercent) }
}

/// Time since the user last touched the keyboard or mouse (anywhere in the session).
fn idle() -> Duration {
    let mut info = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return Duration::ZERO;
    }
    Duration::from_millis(u64::from(unsafe { GetTickCount() }.wrapping_sub(info.dwTime)))
}

/// F15 down and up: input as far as Windows (and Teams) is concerned, which no app acts on.
/// It carries voice's marker, so the dictation hook lets it by.
fn tap_f15() {
    let scan = unsafe { MapVirtualKeyW(u32::from(VK_F15.0), MAPVK_VK_TO_VSC) } as u16;
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VK_F15, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: mynotch_voice::INPUT_MARKER },
        },
    };
    let taps = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    // Best effort: refused (e.g. a secure desktop), it's tried again in a second.
    let sent = unsafe { SendInput(&taps, size_of::<INPUT>() as i32) };
    if sent == 1 {
        unsafe { SendInput(&taps[1..], size_of::<INPUT>() as i32) };
    }
}

#[cfg(test)]
mod tests {
    use super::{Core, Ended, Length, MAX_MINUTES, NOTE, Power, STEP};
    use std::time::{Duration, Instant};

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn slider_stops_every_fifteen_minutes_to_eight_hours_then_no_limit() {
        assert_eq!(Length::at(0.0), Length::Minutes(15));
        assert_eq!(Length::at(-1.0), Length::Minutes(15));
        assert_eq!(Length::at(1.0), Length::Forever);
        assert_eq!(Length::at(31.0 / 32.0), Length::Minutes(MAX_MINUTES));
        for m in (STEP..=MAX_MINUTES).step_by(STEP as usize) {
            assert_eq!(Length::at(Length::Minutes(m).fraction()), Length::Minutes(m), "{m} min reads back");
        }
        assert_eq!(Length::Forever.fraction(), 1.0);
        for k in [0.07, 0.33, 0.5, 0.81] {
            assert!(matches!(Length::at(k), Length::Minutes(m) if m % STEP == 0), "{k}");
        }
    }

    #[test]
    fn lengths_read_and_save() {
        assert_eq!(Length::Minutes(45).label(), "45 m");
        assert_eq!(Length::Minutes(120).label(), "2 h");
        assert_eq!(Length::Minutes(135).label(), "2 h 15 m");
        assert_eq!(Length::Forever.label(), "No limit");
        for l in [Length::Minutes(15), Length::Minutes(480), Length::Forever] {
            assert_eq!(Length::parse(&l.key()), Some(l));
        }
        assert_eq!(Length::parse("50"), Some(Length::Minutes(45)), "to the nearest step");
        assert_eq!(Length::parse("53"), Some(Length::Minutes(60)));
        assert_eq!(Length::parse("0"), Some(Length::Minutes(15)));
        assert_eq!(Length::parse("9999"), Some(Length::Minutes(MAX_MINUTES)));
        assert_eq!(Length::parse("soon"), None);
        assert_eq!(Length::default(), Length::Minutes(60));
    }

    #[test]
    fn a_session_runs_its_length_then_ends_with_a_note() {
        let t0 = Instant::now();
        let mut core = Core::default();
        assert!(!core.on());
        core.start(t0);
        assert_eq!(core.remaining(t0 + 10 * MIN), Some(50 * MIN));
        assert!(!core.tick(t0 + 59 * MIN));
        assert!(core.tick(t0 + 60 * MIN), "time's up");
        assert!(!core.on());
        assert_eq!(core.note(t0 + 60 * MIN), Some(Ended::TimeUp));
        assert_eq!(core.note(t0 + 60 * MIN + NOTE), None, "the note goes after a few seconds");
        assert!(!core.tick(t0 + 61 * MIN), "ends once");
    }

    #[test]
    fn no_limit_runs_until_turned_off() {
        let t0 = Instant::now();
        let mut core = Core { length: Length::Forever, ..Core::default() };
        core.start(t0);
        assert_eq!(core.remaining(t0), None);
        assert!(!core.tick(t0 + 1000 * MIN));
        assert!(core.on());
        core.stop();
        assert!(!core.on());
        assert_eq!(core.note(t0 + 1000 * MIN), None, "turned off by hand: no note");
    }

    #[test]
    fn a_new_length_while_on_counts_from_now() {
        let t0 = Instant::now();
        let mut core = Core::default();
        core.start(t0);
        assert!(core.set_length(Length::Minutes(30), t0 + 50 * MIN));
        assert_eq!(core.remaining(t0 + 50 * MIN), Some(30 * MIN));
        assert!(core.set_length(Length::Minutes(30), t0 + 70 * MIN), "the same length again still restarts it");
        assert_eq!(core.remaining(t0 + 70 * MIN), Some(30 * MIN));
        core.stop();
        assert!(!core.set_length(Length::Minutes(30), t0), "off and unchanged: nothing to draw");
        assert!(!core.on(), "choosing a length doesn't turn it on");
    }

    #[test]
    fn starting_again_clears_the_note() {
        let t0 = Instant::now();
        let mut core = Core::default();
        core.end(Ended::LowBattery, t0);
        assert_eq!(core.note(t0), Some(Ended::LowBattery));
        core.start(t0);
        assert_eq!(core.note(t0), None);
    }

    #[test]
    fn battery_is_low_only_unplugged_below_twenty_percent() {
        let p = |plugged_in, percent| Power { plugged_in, percent }.low();
        assert!(p(false, Some(19)));
        assert!(!p(false, Some(20)));
        assert!(!p(true, Some(5)), "plugged in");
        assert!(!p(false, None), "no battery, or unknown");
    }
}
