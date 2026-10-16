//! Widgets are the panels hosted by the expanded notch; one is shown at a time, picked by
//! the tab row. Each also may offer a one-line readout for the collapsed notch, and results
//! for the tab-row search.

mod apps;
mod clipboard;
mod clock;
mod focus;
mod media;
mod sky;
mod stopwatch;
mod system;
mod time;

use std::rc::Rc;
use std::time::Instant;

use crate::gfx::{Align, Canvas, Font, Ink, Rect};
use crate::motion;
use crate::theme::{motion as timing, space};
use crate::settings::Settings;
use crate::weather::Weather;

/// A round icon button. `primary` is lit cyan glass with a soft halo; others are bare icons
/// that light up under the cursor. Disabled buttons are drawn faint and never highlight.
fn button(cv: &Canvas, r: Rect, icon: &str, font: Font, enabled: bool, primary: bool) {
    let hot = cv.hover_if(r, enabled);
    let (cx, cy, radius) = (r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0);
    if primary {
        cv.circle(cx, cy, radius, Ink::Lit, None);
        cv.circle_a(cx, cy, radius, Ink::Lit, None, hot);
        cv.circle_a(cx, cy, radius - 0.5, Ink::LitEdge, Some(1.0), 0.8 + 0.2 * hot);
        cv.halo(cx, cy, radius, Ink::Accent, 0.5 + 0.5 * hot);
    } else {
        cv.circle_a(cx, cy, radius, Ink::Hover, None, hot);
    }
    if enabled {
        cv.text(icon, font, if primary { Ink::Text } else { Ink::Dim }, r, Align::Center);
        cv.text_a(icon, font, Ink::Text, r, Align::Center, hot);
    } else {
        cv.text(icon, font, Ink::Track, r, Align::Center);
    }
}

/// A rounded rectangular button with an icon and a label, e.g. the stopwatch's Start.
/// `lit` (0..=1) edges it in cyan.
fn pill(cv: &Canvas, r: Rect, icon: &str, label: &str, lit: f32) {
    let hot = cv.hover(r);
    cv.fill(r, space::CONTROL_RADIUS, Ink::Track);
    cv.fill_a(r, space::CONTROL_RADIUS, Ink::Hover, hot);
    cv.fill_a(r, space::CONTROL_RADIUS, Ink::Lit, lit);
    cv.outline(r, space::CONTROL_RADIUS, Ink::Edge, 1.0 - lit);
    cv.outline(r, space::CONTROL_RADIUS, Ink::LitEdge, lit);
    let icon_r = Rect::new(r.x + 8.0, r.y, 18.0, r.h);
    cv.text(icon, Font::Icon, Ink::Text, icon_r, Align::Center);
    let text_r = Rect::new(icon_r.right() + 4.0, r.y, r.w - icon_r.right() + r.x - 10.0, r.h);
    cv.text(label, Font::Label, Ink::Text, text_r, Align::Left);
}

/// One soft ring spreading out from a circle and fading, `since` a timer started. Nothing
/// once it's over, or with animations turned off.
fn pulse(cv: &Canvas, cx: f32, cy: f32, radius: f32, ink: Ink, since: Option<Instant>) {
    let Some(since) = since.filter(|_| !motion::reduced()) else { return };
    let t = since.elapsed().as_secs_f32() / timing::PULSE.as_secs_f32();
    if t < 1.0 {
        let out = 1.0 - (1.0 - t) * (1.0 - t);
        cv.circle_a(cx, cy, radius + 14.0 * out, ink, Some(2.0 + 4.0 * (1.0 - t)), 0.45 * (1.0 - t));
    }
}

/// Whether a pulse started at `since` is still under way, so its widget keeps redrawing.
fn pulsing(since: Option<Instant>) -> bool {
    since.is_some_and(|s| !motion::reduced() && s.elapsed() < timing::PULSE)
}

/// How long the light takes to run once round a running card, in seconds.
const LAP: f32 = 1.0;

/// The cue that a card's timer is running: a light running round its edge. Each lap eases
/// in and out (slow away from the start, fast round the far side, slow into the finish),
/// and its tail stretches with its speed, so it moves elastically. (With animations off,
/// the card's own lit edge says it instead.)
pub fn running_edge(cv: &Canvas, card: Rect, ink: Ink) {
    if !motion::reduced() {
        let t = motion::cycle(LAP) * std::f32::consts::PI;
        let (head, speed) = (0.5 - 0.5 * t.cos(), t.sin());
        cv.runner(card, space::CARD_RADIUS, head, ink, 1.0, 0.35 + 1.15 * speed);
    }
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
    let cy = track.y + track.h / 2.0;
    cv.fill(track, track.h / 2.0, Ink::Track);
    cv.fill(Rect { w: x - track.x, ..track }, track.h / 2.0, if enabled { Ink::Accent } else { Ink::Dim });
    let grow = cv.ease(motion::key(("knob", track.x as i32, track.y as i32)), f32::from(u8::from(hot && enabled)), timing::HOVER);
    let r = KNOB_R * 0.75 + grow;
    if enabled {
        cv.halo(x, cy, r, Ink::Accent, 0.4 + 0.6 * grow);
    }
    cv.circle(x, cy, r, if enabled { Ink::Text } else { Ink::Dim }, None);
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
    /// How far through it is (a running timer), shown as a hairline along the rail.
    pub progress: Option<f32>,
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

    /// Whether something this widget shows is moving on its own (a running timer), so the
    /// shell keeps drawing frames while it's on screen.
    fn animating(&self) -> bool {
        false
    }

    /// `animating`, for the Home card.
    fn card_animating(&self) -> bool {
        false
    }

    /// Whether the Home card is edged in light, as the one that matters most.
    fn card_lit(&self) -> bool {
        false
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

pub use clock::{now_short, weather_icon};

/// Every widget, in tab order. The Time tab shows `weather`.
pub fn all(weather: &Rc<Weather>) -> Vec<Box<dyn Widget>> {
    vec![
        Box::new(media::MediaWidget::new()),
        Box::new(time::TimeWidget::new(weather.clone())),
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
