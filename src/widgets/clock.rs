//! The Time tab's clock card: the time, to the second, and today's date, in the user's own
//! formats.

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

use crate::gfx::{Align, Canvas, Font, Ink, Rect};

/// Room between the digits and the AM/PM marker.
const MARKER_GAP: f32 = 5.0;

/// A digital clock with seconds, above the weekday and date.
pub struct Clock {
    /// The local time last formatted, to the second.
    shown: Option<(u16, u16, u16, u16, u16, u16)>,
    /// "3:04:05", or "15:04:05" with a 24-hour clock.
    time: String,
    /// "PM"; empty with a 24-hour clock.
    marker: String,
    /// "Thursday".
    weekday: String,
    /// "October 15".
    date: String,
}

impl Clock {
    pub fn new() -> Self {
        Self { shown: None, time: String::new(), marker: String::new(), weekday: String::new(), date: String::new() }
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
        if uses_24_hour() {
            self.time = format_time(&st, w!("HH:mm:ss"));
            self.marker.clear();
        } else {
            self.time = format_time(&st, w!("h:mm:ss"));
            self.marker = format_time(&st, w!("tt"));
        }
        self.weekday = format_date(&st, w!("dddd"));
        // The locale's month-and-day picture, e.g. "MMMM dd" (DATE_MONTHDAY is refused here).
        let mut picture = [0u16; 32];
        let n = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_SMONTHDAY, Some(&mut picture)) };
        let picture = if n > 1 { PCWSTR(picture.as_ptr()) } else { w!("MMMM d") };
        self.date = format_date(&st, picture);
        true
    }

    pub fn draw(&self, cv: &Canvas, w: f32, _h: f32) {
        cv.text("Clock", Font::Small, Ink::Dim, Rect::new(0.0, 0.0, w, 16.0), Align::Left);
        // A little slack keeps the measured width from tripping the ellipsis.
        let time_w = cv.measure(&self.time, Font::Display) + 2.0;
        cv.text(&self.time, Font::Display, Ink::Text, Rect::new(0.0, 18.0, time_w, 42.0), Align::Left);
        // Sits low, roughly on the digits' baseline.
        let marker = Rect::new(time_w + MARKER_GAP, 34.0, w - time_w - MARKER_GAP, 20.0);
        cv.text(&self.marker, Font::Label, Ink::Dim, marker, Align::Left);
        cv.text(&self.weekday, Font::Title, Ink::Text, Rect::new(0.0, 76.0, w, 22.0), Align::Left);
        cv.text(&self.date, Font::Body, Ink::Dim, Rect::new(0.0, 98.0, w, 20.0), Align::Left);
    }
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
