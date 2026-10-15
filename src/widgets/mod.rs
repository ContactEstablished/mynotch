//! Widgets are the panels hosted by the expanded notch; one is shown at a time, picked by
//! the tab row. Each also may offer a one-line readout for the collapsed notch.

mod media;
mod system;

use std::time::Instant;

use crate::gfx::Canvas;

/// A one-line readout for the collapsed notch. The widget with the highest `priority` wins.
pub struct Compact {
    pub priority: u8,
    pub text: String,
}

pub trait Widget {
    /// Tab label.
    fn title(&self) -> &'static str;

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
}

/// Every widget, in tab order.
pub fn all() -> Vec<Box<dyn Widget>> {
    vec![Box::new(media::MediaWidget::new()), Box::new(system::SystemWidget::new())]
}
