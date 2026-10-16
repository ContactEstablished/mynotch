//! The Time tab's clock card: the time, to the second, and today's date, in the user's own
//! formats, under the sun or the moon (in its phase), with the local weather; also the time
//! for the collapsed rail.

use std::rc::Rc;

use windows::{
    Win32::{
        Foundation::SYSTEMTIME,
        Globalization::{
            ENUM_DATE_FORMATS_FLAGS, GetDateFormatEx, GetLocaleInfoEx, GetTimeFormatEx, LOCALE_ITIME, LOCALE_SMONTHDAY,
            TIME_FORMAT_FLAGS,
        },
        System::SystemInformation::GetLocalTime,
    },
    core::{PCWSTR, w},
};

use super::sky;
use crate::gfx::{Align, Canvas, Font, Ink, Rect};
use crate::theme::color;
use crate::weather::{self, Status, Weather};

/// Room between the digits and the AM/PM marker.
const MARKER_GAP: f32 = 5.0;

/// A digital clock with seconds, above the weekday and date, the weather, and a line showing
/// how far through the day (or night) it is.
pub struct Clock {
    weather: Rc<Weather>,
    /// The local time last formatted, to the second.
    shown: Option<(u16, u16, u16, u16, u16, u16)>,
    /// "3:04", or "15:04" with a 24-hour clock.
    time: String,
    /// ":05".
    seconds: String,
    /// "PM"; empty with a 24-hour clock.
    marker: String,
    /// "Thursday".
    weekday: String,
    /// "October 15".
    date: String,
    /// Minutes since local midnight: day and night by the clock when there are no sun times.
    minutes: u32,
}

impl Clock {
    pub fn new(weather: Rc<Weather>) -> Self {
        Self {
            weather,
            shown: None,
            time: String::new(),
            seconds: String::new(),
            marker: String::new(),
            weekday: String::new(),
            date: String::new(),
            minutes: 0,
        }
    }

    /// Re-formats once a second. Returns true when the text changed.
    pub fn update(&mut self) -> bool {
        let st = unsafe { GetLocalTime() };
        let key = (st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond);
        if self.shown == Some(key) {
            return false;
        }
        self.shown = Some(key);
        // Seconds are always shown, so build the picture rather than use the user's long
        // time format (which may leave them out), honouring only their 12/24-hour choice.
        (self.time, self.marker) = short_time(&st);
        self.seconds = format_time(&st, w!(":ss"));
        self.weekday = format_date(&st, w!("dddd"));
        // The locale's month-and-day picture, e.g. "MMMM dd" (DATE_MONTHDAY is refused here).
        let mut picture = [0u16; 32];
        let n = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_SMONTHDAY, Some(&mut picture)) };
        let picture = if n > 1 { PCWSTR(picture.as_ptr()) } else { w!("MMMM d") };
        self.date = format_date(&st, picture);
        self.minutes = u32::from(st.wHour) * 60 + u32::from(st.wMinute);
        true
    }

    /// Day or night, and how far through it: from the place's sunrises and sunsets when the
    /// weather has them, else by the clock (day from 6:00 to 18:00).
    fn daylight(&self, report: Option<&weather::Report>, now: i64) -> (bool, f32) {
        report.and_then(|r| weather::daylight(&r.sun, now)).unwrap_or_else(|| {
            let from_six = (self.minutes + 24 * 60 - 6 * 60) % (24 * 60);
            (from_six < 12 * 60, (from_six % (12 * 60)) as f32 / (12.0 * 60.0))
        })
    }

    pub fn draw(&self, cv: &Canvas, w: f32, _h: f32) {
        let now = weather::unix_now();
        let report = self.weather.report();
        let (day, through) = self.daylight(report.as_ref(), now);
        let (age, south) = (weather::moon_age(now), report.as_ref().is_some_and(|r| r.south));

        // Heading: the sun or the moon, then the label; the weather on the right.
        sky::body(cv, (7.0, 7.0), 6.5, day, age, south);
        super::time::caption(cv, 21.0, "Local time", Ink::Dim, w);
        if let Some(r) = &report {
            // Rounded, and never "-0°".
            let degrees = r.temperature.round() + 0.0;
            let temperature = format!("{degrees:.0}°");
            let tw = cv.numerals(&[(temperature.as_str(), Font::Numeral, Ink::Text)], w, 11.5, Align::Right);
            // A clear sky is already the sun or moon at the left.
            if r.sky != weather::Sky::Clear || r.windy {
                sky::weather(cv, (w - tw - 12.0, 7.0), 17.0, r.sky, r.windy, day, age, south);
            }
        }

        let baseline = 56.0;
        let runs = [(self.time.as_str(), Font::Clock, Ink::Text), (self.seconds.as_str(), Font::Fraction, Ink::Dim)];
        let width = cv.numerals(&runs, 0.0, baseline, Align::Left);
        if !self.marker.is_empty() {
            let marker = Rect::new(width + MARKER_GAP, baseline - 13.0, w - width - MARKER_GAP, 14.0);
            cv.caps(&self.marker, Font::Caption, Ink::Dim, marker, Align::Left);
        }
        let row = Rect::new(0.0, 62.0, w, 20.0);
        cv.text(&self.weekday, Font::Label, Ink::Text, row, Align::Left);
        let weekday_w = cv.measure(&self.weekday, Font::Label) + 8.0;
        cv.text(&self.date, Font::Small, Ink::Dim, Rect { x: weekday_w, w: w - weekday_w, ..row }, Align::Right);

        // The weather in words, or what's keeping it.
        let words = match (&report, self.weather.status()) {
            (Some(r), _) if r.windy => format!("{} · Windy", r.summary),
            (Some(r), _) => r.summary.to_owned(),
            (None, Status::Off) => "Add your city in Settings".to_owned(),
            (None, Status::NotFound) => "Couldn't find that place".to_owned(),
            (None, Status::Offline) => "Weather unavailable".to_owned(),
            (None, _) => "Getting the weather…".to_owned(),
        };
        cv.text(&words, Font::Small, Ink::Dim, Rect::new(0.0, 82.0, w, 18.0), Align::Left);

        // How far through the day (sun gold) or the night (moon violet): a hairline with a
        // lit stretch ending in a dot.
        let (y, ink) = (108.0, if day { Ink::Rgb(color::SUN) } else { Ink::Violet });
        cv.line((0.0, y), (w, y), Ink::Line);
        let x = (w * through).max(2.0);
        cv.fill_a(Rect::new(0.0, y - 0.75, x, 1.5), 0.75, ink, 0.85);
        cv.halo(x, y, 2.0, ink, 0.5);
        cv.circle(x, y, 2.0, ink, None);
    }
}

/// The time to the minute, as ("9:08", "PM"), or ("21:08", "") with a 24-hour clock.
fn short_time(st: &SYSTEMTIME) -> (String, String) {
    if uses_24_hour() {
        (format_time(st, w!("HH:mm")), String::new())
    } else {
        (format_time(st, w!("h:mm")), format_time(st, w!("tt")))
    }
}

/// The local time `from_now` from now, to the minute: "3:42 PM", or "15:42".
pub fn time_after(from_now: std::time::Duration) -> String {
    let mut st = unsafe { GetLocalTime() };
    let secs = (u64::from(st.wHour) * 3600 + u64::from(st.wMinute) * 60 + u64::from(st.wSecond) + from_now.as_secs()) % 86_400;
    (st.wHour, st.wMinute, st.wSecond) = ((secs / 3600) as u16, (secs / 60 % 60) as u16, (secs % 60) as u16);
    let (time, marker) = short_time(&st);
    if marker.is_empty() { time } else { format!("{time} {marker}") }
}

/// The time now, to the minute, for the collapsed rail: ("9:08", "PM").
pub fn now_short() -> (String, String) {
    short_time(&unsafe { GetLocalTime() })
}

/// Whether the user's regional settings use a 24-hour clock.
fn uses_24_hour() -> bool {
    let mut buf = [0u16; 4];
    // A null locale name means the user's default.
    let n = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_ITIME, Some(&mut buf)) };
    n > 0 && buf[0] == u16::from(b'1')
}

/// The text of a formatting call's buffer; `n` counts the terminating null, and 0 means it
/// failed.
fn written(buf: &[u16], n: i32) -> String {
    String::from_utf16_lossy(&buf[..(n.max(1) as usize - 1).min(buf.len())])
}

/// Formats `st` with a time picture such as "h:mm:ss" in the user's locale.
fn format_time(st: &SYSTEMTIME, picture: PCWSTR) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetTimeFormatEx(PCWSTR::null(), TIME_FORMAT_FLAGS(0), Some(st), picture, Some(&mut buf)) };
    written(&buf, n)
}

/// Formats `st` with a date picture such as "dddd" in the user's locale.
fn format_date(st: &SYSTEMTIME, picture: PCWSTR) -> String {
    let mut buf = [0u16; 128];
    let flags = ENUM_DATE_FORMATS_FLAGS(0);
    let n = unsafe { GetDateFormatEx(PCWSTR::null(), flags, Some(st), picture, Some(&mut buf), PCWSTR::null()) };
    written(&buf, n)
}
