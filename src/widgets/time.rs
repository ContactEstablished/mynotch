//! The Time tab: the clock, the focus timer and a stopwatch side by side, as cards.

use std::time::Instant;

use super::clock::Clock;
use super::focus::Focus;
use super::stopwatch::Stopwatch;
use super::{Compact, Widget};
use crate::gfx::{Canvas, Ink, Rect, glyph};
use crate::settings::Settings;

const GAP: f32 = 12.0;
const PAD: f32 = 14.0;
const RADIUS: f32 = 16.0;
/// Shares of the row for the clock, focus and stopwatch cards; focus has the most to show.
const SHARES: [f32; 3] = [0.3, 0.4, 0.3];

/// The cards' rects in a `w` x `h` content area.
fn cards(w: f32, h: f32) -> [Rect; 3] {
    let free = w - GAP * 2.0;
    let mut x = 0.0;
    SHARES.map(|k| {
        let r = Rect::new(x, 0.0, free * k, h);
        x += r.w + GAP;
        r
    })
}

/// A card's content area.
fn inner(card: Rect) -> Rect {
    Rect::new(card.x + PAD, card.y + PAD, card.w - PAD * 2.0, card.h - PAD * 2.0)
}

pub struct TimeWidget {
    clock: Clock,
    focus: Focus,
    stopwatch: Stopwatch,
    /// Content size at the last press, for following a drag.
    size: (f32, f32),
}

impl TimeWidget {
    pub fn new() -> Self {
        Self { clock: Clock::new(), focus: Focus::new(), stopwatch: Stopwatch::new(), size: (672.0, 176.0) }
    }
}

impl Widget for TimeWidget {
    fn title(&self) -> &'static str {
        "Time"
    }

    fn icon(&self) -> &'static str {
        glyph::CLOCK
    }

    fn update(&mut self, now: Instant) -> bool {
        // All three, every frame (no short-circuit).
        self.clock.update() | self.focus.update(now) | self.stopwatch.update(now)
    }

    fn compact(&self) -> Option<Compact> {
        [self.focus.compact(), self.stopwatch.compact()].into_iter().flatten().max_by_key(|c| c.priority)
    }

    fn draw(&self, cv: &Canvas, w: f32, h: f32) {
        let [clock, focus, stopwatch] = cards(w, h);
        for card in [clock, focus, stopwatch] {
            cv.fill(card, RADIUS, Ink::Track);
        }
        let at = |card: Rect| {
            let r = inner(card);
            (cv.offset(r.x, r.y), r.w, r.h)
        };
        let (c, w0, h0) = at(clock);
        self.clock.draw(&c, w0, h0);
        let (c, w1, h1) = at(focus);
        self.focus.draw(&c, w1, h1);
        let (c, w2, h2) = at(stopwatch);
        self.stopwatch.draw(&c, w2, h2);
    }

    fn click(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let [_, focus, stopwatch] = cards(w, h).map(inner);
        let local = |r: Rect| (x - r.x, y - r.y);
        if focus.contains(x, y) {
            let (x, y) = local(focus);
            self.focus.click(x, y, focus.w, focus.h)
        } else if stopwatch.contains(x, y) {
            let (x, y) = local(stopwatch);
            self.stopwatch.click(x, y, stopwatch.w, stopwatch.h)
        } else {
            false
        }
    }

    fn press(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        self.size = (w, h);
        let focus = inner(cards(w, h)[1]);
        self.focus.press(x - focus.x, y - focus.y, focus.w, focus.h)
    }

    fn drag(&mut self, x: f32, _y: f32) -> bool {
        let focus = inner(cards(self.size.0, self.size.1)[1]);
        self.focus.drag(x - focus.x, focus.w, focus.h)
    }

    fn load(&mut self, settings: &Settings) {
        self.focus.load(settings);
    }

    fn store(&self, settings: &mut Settings) {
        self.focus.store(settings);
    }

    fn take_attention(&mut self) -> bool {
        self.focus.take_attention()
    }

    fn card_weight(&self) -> Option<f32> {
        Some(1.0)
    }

    fn draw_card(&self, cv: &Canvas, w: f32, h: f32) {
        self.focus.draw_card(cv, w, h);
    }

    fn click_card(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        self.focus.click_card(x, y, w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::{GAP, cards};

    #[test]
    fn cards_fill_the_row_with_focus_widest() {
        let [clock, focus, stopwatch] = cards(672.0, 176.0);
        assert_eq!(clock.x, 0.0);
        assert!((stopwatch.right() - 672.0).abs() < 0.01);
        assert!((focus.x - clock.right() - GAP).abs() < 0.01 && (stopwatch.x - focus.right() - GAP).abs() < 0.01);
        assert!(focus.w > clock.w && focus.w > stopwatch.w);
    }
}
