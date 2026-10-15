//! Widgets are the panels hosted by the expanded notch; one is shown at a time, picked by
//! the tab row. Each also may offer a one-line readout for the collapsed notch, and results
//! for the tab-row search.

mod apps;
mod clipboard;
mod clock;
mod focus;
mod media;
mod stopwatch;
mod system;
mod time;

use std::time::Instant;

use crate::gfx::{Align, Canvas, Font, Ink, Rect};
use crate::settings::Settings;

/// A round icon button. `primary` gets a filled disc; others only light up on hover.
/// Disabled buttons are drawn faint and never highlight.
fn button(cv: &Canvas, r: Rect, icon: &str, font: Font, enabled: bool, primary: bool) {
    let hot = enabled && cv.hovered(r);
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    if primary {
        cv.circle(cx, cy, r.w / 2.0, if hot { Ink::Hover } else { Ink::Track }, None);
    } else if hot {
        cv.circle(cx, cy, r.w / 2.0, Ink::Track, None);
    }
    cv.text(icon, font, if enabled { Ink::Text } else { Ink::Track }, r, Align::Center);
}

/// Square buttons of the given sizes in a row centred on `cx`, vertically centred on `cy`.
fn button_row<const N: usize>(cx: f32, cy: f32, sizes: [f32; N], gap: f32) -> [Rect; N] {
    let total: f32 = sizes.iter().sum::<f32>() + gap * (N.saturating_sub(1)) as f32;
    let mut x = cx - total / 2.0;
    sizes.map(|s| {
        let r = Rect::new(x, cy - s / 2.0, s, s);
        x += s + gap;
        r
    })
}

const KNOB_R: f32 = 8.0;

/// A horizontal slider: `track` filled up to the knob at fraction `k` of its length.
/// Disabled sliders are drawn grey.
pub fn slider(cv: &Canvas, track: Rect, k: f32, hot: bool, enabled: bool) {
    let x = track.x + track.w * k.clamp(0.0, 1.0);
    cv.fill(track, track.h / 2.0, Ink::Track);
    cv.fill(Rect { w: x - track.x, ..track }, track.h / 2.0, if enabled { Ink::Accent } else { Ink::Dim });
    let r = if hot && enabled { KNOB_R + 1.0 } else { KNOB_R };
    cv.circle(x, track.y + track.h / 2.0, r, if enabled { Ink::Text } else { Ink::Dim }, None);
}

/// The fraction of `track`'s length at x, for a slider.
pub fn slider_at(track: Rect, x: f32) -> f32 {
    ((x - track.x) / track.w).clamp(0.0, 1.0)
}

/// Generous hit area around a slider's track.
pub fn slider_hit(track: Rect) -> Rect {
    Rect::new(track.x - KNOB_R * 2.0, track.y - 14.0, track.w + KNOB_R * 4.0, track.h + 28.0)
}

/// "4:05", or "1:02:03" past an hour.
fn clock(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// A one-line readout for the collapsed notch. The widget with the highest `priority` wins.
pub struct Compact {
    pub priority: u8,
    pub text: String,
    /// Live activity (e.g. a song playing): opening the notch then shows this widget instead
    /// of the last tab the user picked.
    pub live: bool,
}

/// Keys the shell forwards to the widget showing search results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchKey {
    /// Down or Tab: the next result in reading order.
    Next,
    /// Up or Shift+Tab.
    Previous,
    /// Open the highlighted result.
    Enter,
}

/// What a search key or click did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchAction {
    None,
    /// The results changed; redraw.
    Redraw,
    /// A result was opened (e.g. an app launched): the shell closes search without giving
    /// the focus back, so what was opened can come to the front.
    Done,
}

pub trait Widget {
    /// Name, shown while its tab is hovered or open; also how the last tab is remembered.
    fn title(&self) -> &'static str;

    /// Tab icon, from `glyph`.
    fn icon(&self) -> &'static str;

    /// Advances internal state. Called every frame, so throttle expensive work.
    /// Returns true when the widget needs a redraw.
    fn update(&mut self, now: Instant) -> bool;

    fn compact(&self) -> Option<Compact>;

    /// Draws the widget's content area, of logical size `w` x `h`, at the canvas origin.
    fn draw(&self, cv: &Canvas, w: f32, h: f32);

    /// A left click at logical content-area coordinates. Returns true when the widget
    /// needs a redraw.
    fn click(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) -> bool {
        false
    }

    /// Left button down, before `click`. Returning true starts a drag instead of a click:
    /// the shell holds the mouse, calls `drag` as it moves and `release` when the button
    /// comes up, then saves preferences.
    fn press(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) -> bool {
        false
    }

    /// The mouse moved during a drag this widget started, in content-area coordinates (it
    /// may be outside the panel). Returns true when the widget needs a redraw.
    fn drag(&mut self, _x: f32, _y: f32) -> bool {
        false
    }

    /// The button came up, ending the drag, in content-area coordinates.
    fn release(&mut self, _x: f32, _y: f32) {}

    /// Restores this widget's preferences. Called once at startup.
    fn load(&mut self, _settings: &Settings) {}

    /// Copies this widget's preferences into `settings`; the shell saves the file if
    /// anything changed. Called after a click the widget handled.
    fn store(&self, _settings: &mut Settings) {}

    /// True once when the widget wants to be seen (e.g. a timer just finished): the notch
    /// briefly opens on it and plays a sound.
    fn take_attention(&mut self) -> bool {
        false
    }

    /// Mouse wheel over the panel, in notches (positive = away from the user). Returns true
    /// when the widget needs a redraw.
    fn wheel(&mut self, _notches: f32) -> bool {
        false
    }

    /// Relative width of this widget's card on the Home dashboard; `None` for no card.
    fn card_weight(&self) -> Option<f32> {
        None
    }

    /// Draws the Home card's content area, of logical size `w` x `h`, at the canvas origin.
    fn draw_card(&self, _cv: &Canvas, _w: f32, _h: f32) {}

    /// A left click on the Home card, in card-content coordinates. Returns true when the
    /// widget needs a redraw.
    fn click_card(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) -> bool {
        false
    }

    /// The tab-row search's query changed; also called with "" whenever search opens.
    /// Returns true when this widget offers results for it; the shell shows the first widget
    /// (in `all()` order) that does. There's no end-of-search call: the next `search`
    /// replaces whatever state this one kept.
    fn search(&mut self, _query: &str) -> bool {
        false
    }

    /// Draws this widget's search results over the content area, `w` x `h`, at the canvas
    /// origin.
    fn draw_search(&self, _cv: &Canvas, _w: f32, _h: f32) {}

    /// A navigation key while this widget's results are shown.
    fn search_key(&mut self, _key: SearchKey) -> SearchAction {
        SearchAction::None
    }

    /// A left click on the results, in content-area coordinates.
    fn click_search(&mut self, _x: f32, _y: f32, _w: f32, _h: f32) -> SearchAction {
        SearchAction::None
    }

    /// Mouse wheel over the results, in notches. Returns true when it needs a redraw.
    fn wheel_search(&mut self, _notches: f32) -> bool {
        false
    }
}

/// Every widget, in tab order.
pub fn all() -> Vec<Box<dyn Widget>> {
    vec![
        Box::new(media::MediaWidget::new()),
        Box::new(time::TimeWidget::new()),
        Box::new(apps::AppsWidget::new()),
        Box::new(clipboard::ClipboardWidget::new()),
        Box::new(system::SystemWidget::new()),
    ]
}

#[cfg(test)]
mod tests {
    use super::{button_row, clock};
    use std::time::Duration;

    #[test]
    fn clock_format() {
        assert_eq!(clock(Duration::from_secs(25 * 60)), "25:00");
        assert_eq!(clock(Duration::from_secs(245)), "4:05");
        assert_eq!(clock(Duration::from_secs(3723)), "1:02:03");
    }

    #[test]
    fn button_row_is_centred() {
        let [a, b] = button_row(100.0, 50.0, [20.0, 40.0], 10.0);
        assert_eq!((a.x, a.y, b.x, b.y), (65.0, 40.0, 95.0, 30.0));
        assert_eq!(b.x + b.w - 100.0, 100.0 - a.x, "symmetric about the centre");
    }
}
