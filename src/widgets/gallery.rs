//! The timer style gallery: every style side by side, each showing the timer as it stands,
//! to pick from. Opened from the Timer card's palette button; it covers the Time tab until
//! a style is picked or it's closed.

use super::button;
use super::styles::{Dial, Face, Size, Style};
use crate::gfx::{Align, Canvas, Font, Ink, Rect, glyph};
use crate::motion;
use crate::theme::{motion as timing, space};

const COLS: usize = 4;
/// Where the tiles start, under the heading.
const TOP: f32 = 22.0;
const CLOSE: f32 = 22.0;
const TILE_PAD: f32 = 10.0;
/// The tile's ring: its radius and how far its centre sits in from the tile's right.
const RING_R: f32 = 14.0;
const RING_IN: f32 = 16.0;

/// What a click in the gallery landed on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    Style(Style),
    Close,
}

/// The timer as the tiles show it.
pub struct Preview<'a> {
    pub face: Face<'a>,
    /// The fraction of the time still to go.
    pub left: f32,
}

/// Each style's tile in a `w` x `h` content area: four to a row, under the heading.
fn tiles(w: f32, h: f32) -> impl Iterator<Item = (Style, Rect)> {
    let rows = Style::ALL.len().div_ceil(COLS);
    let tile_w = (w - space::GAP * (COLS - 1) as f32) / COLS as f32;
    let tile_h = (h - TOP - space::GAP * (rows - 1) as f32) / rows as f32;
    Style::ALL.into_iter().enumerate().map(move |(i, style)| {
        let (col, row) = ((i % COLS) as f32, (i / COLS) as f32);
        (style, Rect::new(col * (tile_w + space::GAP), TOP + row * (tile_h + space::GAP), tile_w, tile_h))
    })
}

fn close(w: f32) -> Rect {
    Rect::new(w - CLOSE, -4.0, CLOSE, CLOSE)
}

pub fn hit(x: f32, y: f32, w: f32, h: f32) -> Option<Pick> {
    if close(w).contains(x, y) {
        return Some(Pick::Close);
    }
    tiles(w, h).find(|(_, r)| r.contains(x, y)).map(|(style, _)| Pick::Style(style))
}

pub fn draw(cv: &Canvas, w: f32, h: f32, chosen: Style, preview: &Preview) {
    super::time::header(cv, Some(glyph::PALETTE), "Timer style", Ink::Accent, w);
    let hint = Rect::new(0.0, -2.0, w - CLOSE - 8.0, 16.0);
    cv.text("Pick a look for the timer", Font::Small, Ink::Dim, hint, Align::Right);
    button(cv, close(w), glyph::CLOSE, Font::IconCaption, true, false);

    for (style, r) in tiles(w, h) {
        let picked = style == chosen;
        let lit = cv.ease(motion::key(("style-tile", style.key())), f32::from(u8::from(picked)), timing::STATE);
        cv.card_in(r, space::CARD_RADIUS, lit.max(0.45 * cv.hover(r)), style.accent());
        let inner = r.inset(TILE_PAD);
        let tile = cv.offset(inner.x, inner.y);
        let name = Rect::new(0.0, -3.0, inner.w - RING_IN - RING_R, 16.0);
        tile.text(style.name(), Font::Label, if picked { Ink::Text } else { Ink::Dim }, name, Align::Left);
        if picked {
            let after = tile.measure(style.name(), Font::Label) + 4.0;
            let check = Rect::new(after, name.y, 16.0, name.h);
            tile.text(glyph::CHECK, Font::IconCaption, Ink::Rgb(style.accent()), check, Align::Left);
        }
        style.time(&tile, &preview.face, 0.0, inner.h - 2.0, Size::Tile);
        let dial = Dial {
            cx: inner.w - RING_IN,
            cy: inner.h / 2.0,
            r: RING_R,
            width: 2.0,
            left: preview.left,
            running: preview.face.running,
            hot: 0.0,
            on: 1.0,
        };
        style.ring(&tile, &dial);
    }
}

#[cfg(test)]
mod tests {
    use super::{Pick, Style, close, hit, tiles};

    #[test]
    fn eight_tiles_fill_the_content_area_without_overlapping() {
        // The Time tab's content area is 692 x 146 (`CONTENT` in app.rs).
        let (w, h) = (692.0, 146.0);
        let all: Vec<_> = tiles(w, h).collect();
        assert_eq!(all.len(), Style::ALL.len());
        for (i, (_, a)) in all.iter().enumerate() {
            assert!(a.x >= 0.0 && a.right() <= w + 0.01 && a.y + a.h <= h + 0.01, "{i} inside");
            assert!(a.y > close(w).y + close(w).h - 4.0, "{i} under the heading");
            for (_, b) in &all[i + 1..] {
                let apart = a.right() <= b.x || b.right() <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
            // Room for a name, a time and a ring.
            assert!(a.w > 150.0 && a.h > 50.0);
        }
    }

    #[test]
    fn clicks_pick_a_style_or_close() {
        let (w, h) = (692.0, 146.0);
        for (style, r) in tiles(w, h) {
            assert_eq!(hit(r.x + r.w / 2.0, r.y + r.h / 2.0, w, h), Some(Pick::Style(style)));
        }
        let c = close(w);
        assert_eq!(hit(c.x + c.w / 2.0, c.y + c.h / 2.0, w, h), Some(Pick::Close));
        assert_eq!(hit(200.0, 4.0, w, h), None, "the heading");
    }
}
