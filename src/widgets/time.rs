//! The Time tab: the focus timer, the clock (with the weather) and a stopwatch side by side,
//! as cards.

use std::rc::Rc;
use std::time::Instant;

use super::clock::Clock;
use super::focus::Focus;
use super::gallery::{self, Pick, Preview};
use super::stopwatch::Stopwatch;
use super::styles::Face;
use super::{Compact, Widget, button, running_edge};
use crate::gfx::{Align, Canvas, Font, Ink, Rect, glyph};
use crate::motion;
use crate::settings::Settings;
use crate::theme::{motion as timing, space};
use crate::weather::Weather;

/// Shares of the row for the timer, clock and stopwatch cards; the clock, in the middle,
/// has the most to show.
const SHARES: [f32; 3] = [0.34, 0.36, 0.30];

/// The cards' rects in a `w` x `h` content area: timer, clock, stopwatch.
fn cards(w: f32, h: f32) -> [Rect; 3] {
    let free = w - space::GAP * 2.0;
    let mut x = 0.0;
    SHARES.map(|k| {
        let r = Rect::new(x, 0.0, free * k, h);
        x += r.w + space::GAP;
        r
    })
}

/// A card's content area.
fn inner(card: Rect) -> Rect {
    card.inset(space::CARD_PAD)
}

/// The Timer card's palette button, which opens the style gallery, in the top right corner
/// of the card's content area (`w` wide).
fn style_button(w: f32) -> Rect {
    Rect::new(w - 18.0, -4.0, 22.0, 22.0)
}

/// A card's heading: an icon (or a dot, in `ink`) and an uppercase label.
pub(super) fn header(cv: &Canvas, icon: Option<&str>, label: &str, ink: Ink, w: f32) {
    let row = Rect::new(0.0, 0.0, w, 14.0);
    match icon {
        Some(icon) => cv.text(icon, Font::IconCaption, ink, Rect { w: 14.0, ..row }, Align::Center),
        None => {
            cv.halo(4.0, 7.0, 2.5, ink, 0.6);
            cv.circle(4.0, 7.0, 2.5, ink, None);
        }
    }
    caption(cv, if icon.is_some() { 21.0 } else { 13.0 }, label, ink, w);
}

/// A card heading's uppercase label from `x`, for cards that draw their own icon.
pub(super) fn caption(cv: &Canvas, x: f32, label: &str, ink: Ink, w: f32) {
    cv.caps(label, Font::Caption, ink, Rect::new(x, -1.0, w - x, 16.0), Align::Left);
}

pub struct TimeWidget {
    focus: Focus,
    clock: Clock,
    stopwatch: Stopwatch,
    /// Content size at the last press, for following a drag.
    size: (f32, f32),
    /// The timer style gallery is open, over the cards.
    picking: bool,
}

impl TimeWidget {
    pub fn new(weather: Rc<Weather>) -> Self {
        Self {
            focus: Focus::new(),
            clock: Clock::new(weather),
            stopwatch: Stopwatch::new(),
            size: (692.0, 146.0),
            picking: false,
        }
    }

    fn draw_gallery(&self, cv: &Canvas, w: f32, h: f32) {
        let (time, left, running) = self.focus.preview(Instant::now());
        let preview = Preview { face: Face { time: &time, flip: None, running }, left };
        gallery::draw(cv, w, h, self.focus.style(), &preview);
    }

    fn click_gallery(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        match gallery::hit(x, y, w, h) {
            Some(Pick::Style(style)) => {
                self.focus.set_style(style);
                self.picking = false;
            }
            Some(Pick::Close) => self.picking = false,
            None => return false,
        }
        true
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
        self.focus.update(now) | self.clock.update() | self.stopwatch.update(now)
    }

    fn compact(&self) -> Option<Compact> {
        [self.focus.compact(), self.stopwatch.compact()].into_iter().flatten().max_by_key(|c| c.priority)
    }

    fn draw(&self, cv: &Canvas, w: f32, h: f32) {
        if self.picking {
            self.draw_gallery(cv, w, h);
            return;
        }
        let [focus, clock, stopwatch] = cards(w, h);
        // The timer is the one that matters: lit always (in its style's colour), brighter
        // while it runs. The stopwatch lights up while it runs. Running ones get a light
        // round their edge.
        let style = self.focus.style();
        let lit = cv.ease(motion::key("focus-card"), if self.focus.running() { 1.0 } else { 0.6 }, timing::STATE);
        let ticking = cv.ease(motion::key("stopwatch-card"), f32::from(u8::from(self.stopwatch.is_running())), timing::STATE);
        cv.card_in(focus, space::CARD_RADIUS, lit, style.accent());
        cv.card(clock, space::CARD_RADIUS, 0.0);
        cv.card(stopwatch, space::CARD_RADIUS, 0.6 * ticking);
        if self.focus.running() {
            style.edge(cv, focus);
        }
        if self.stopwatch.is_running() {
            running_edge(cv, stopwatch, Ink::Accent);
        }
        let at = |card: Rect| {
            let r = inner(card);
            (cv.offset(r.x, r.y), r.w, r.h)
        };
        let (c, w0, h0) = at(focus);
        self.focus.draw(&c, w0, h0);
        button(&c, style_button(w0), glyph::PALETTE, Font::IconCaption, true, false);
        let (c, w1, h1) = at(clock);
        self.clock.draw(&c, w1, h1);
        let (c, w2, h2) = at(stopwatch);
        self.stopwatch.draw(&c, w2, h2);
    }

    fn click(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        if self.picking {
            return self.click_gallery(x, y, w, h);
        }
        let [focus, _, stopwatch] = cards(w, h).map(inner);
        let local = |r: Rect| (x - r.x, y - r.y);
        if style_button(focus.w).contains(x - focus.x, y - focus.y) {
            self.picking = true;
            true
        } else if focus.contains(x, y) {
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
        if self.picking {
            return false;
        }
        self.size = (w, h);
        let focus = inner(cards(w, h)[0]);
        self.focus.press(x - focus.x, y - focus.y, focus.w, focus.h)
    }

    fn drag(&mut self, x: f32, _y: f32) -> bool {
        let focus = inner(cards(self.size.0, self.size.1)[0]);
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

    fn card_lit(&self) -> bool {
        true
    }

    fn draw_card_frame(&self, cv: &Canvas, r: Rect) {
        let style = self.focus.style();
        cv.card_in(r, space::CARD_RADIUS, 0.6, style.accent());
        if self.focus.running() {
            style.edge(cv, r);
        }
    }

    fn leave(&mut self) {
        self.picking = false;
    }

    fn animating(&self) -> bool {
        self.focus.running() || self.stopwatch.is_running()
    }

    fn card_animating(&self) -> bool {
        self.focus.running()
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
    use super::cards;
    use crate::theme::space::GAP;

    #[test]
    fn cards_fill_the_row_with_the_clock_in_the_middle_and_widest() {
        let [timer, clock, stopwatch] = cards(692.0, 146.0);
        assert_eq!(timer.x, 0.0);
        assert!((stopwatch.right() - 692.0).abs() < 0.01);
        assert!((clock.x - timer.right() - GAP).abs() < 0.01 && (stopwatch.x - clock.right() - GAP).abs() < 0.01);
        assert!(clock.w > timer.w && clock.w > stopwatch.w);
        assert!((clock.x + clock.w / 2.0 - 346.0).abs() < 60.0, "the clock sits near the middle");
    }
}
