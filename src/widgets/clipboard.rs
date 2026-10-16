use std::cell::Cell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::{Compact, Widget, button};
use crate::clipboard::{self, Clip, Content, Watcher};
use crate::gfx::{Align, Canvas, Font, Ink, Rect, glyph};

/// Items kept; older ones fall off the end.
const MAX_ITEMS: usize = 30;
const ROW_H: f32 = 34.0;
/// Room on the right for the clear button.
const GUTTER: f32 = 44.0;
/// How long a row says "Copied" after being clicked.
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// Recent clipboard text, links and images; click one to copy it again.
pub struct ClipboardWidget {
    watcher: Option<Watcher>,
    /// Newest first.
    items: VecDeque<Clip>,
    /// First visible row.
    scroll: usize,
    copied: Option<(u64, Instant)>,
    /// When ages ("2m") were last drawn, so they refresh about once a minute.
    aged_at: Instant,
    rows_visible: Cell<usize>,
}

impl ClipboardWidget {
    pub fn new() -> Self {
        Self {
            watcher: Watcher::new(),
            items: VecDeque::new(),
            scroll: 0,
            copied: None,
            aged_at: Instant::now(),
            rows_visible: Cell::new(5),
        }
    }

    fn clear_button(w: f32) -> Rect {
        Rect::new(w - 32.0, 0.0, 32.0, 32.0)
    }

    /// Visible rows as (index into `items`, rect).
    fn rows(&self, w: f32, h: f32) -> impl Iterator<Item = (usize, Rect)> {
        let visible = ((h / ROW_H).floor() as usize).max(1);
        self.rows_visible.set(visible);
        (self.scroll..(self.scroll + visible).min(self.items.len()))
            .map(move |i| (i, Rect::new(0.0, (i - self.scroll) as f32 * ROW_H, w - GUTTER, ROW_H)))
    }
}

/// "now", "5m", "2h", "3d".
fn age(d: Duration) -> String {
    let m = d.as_secs() / 60;
    match m {
        0 => "now".into(),
        1..=59 => format!("{m}m"),
        60..=1439 => format!("{}h", m / 60),
        _ => format!("{}d", m / 1440),
    }
}

impl Widget for ClipboardWidget {
    fn title(&self) -> &'static str {
        "Clipboard"
    }

    fn icon(&self) -> &'static str {
        glyph::CLIPBOARD
    }

    fn update(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if let Some(watcher) = &self.watcher {
            for mut clip in watcher.drain() {
                // A repeat copy (including our own re-copies) moves the item to the top.
                if let Some(i) = self.items.iter().position(|c| c.content.same_as(&clip.content)) {
                    clip.id = self.items[i].id;
                    self.items.remove(i);
                }
                self.items.push_front(clip);
                self.items.truncate(MAX_ITEMS);
                changed = true;
            }
        }
        if self.copied.is_some_and(|(_, at)| now.duration_since(at) > COPIED_FOR) {
            self.copied = None;
            changed = true;
        }
        if now.duration_since(self.aged_at) >= Duration::from_secs(30) {
            self.aged_at = now;
            changed = true;
        }
        changed
    }

    fn compact(&self) -> Option<Compact> {
        None
    }

    fn draw(&self, cv: &Canvas, w: f32, h: f32) {
        if self.watcher.is_none() {
            cv.text("Clipboard history isn't available", Font::Body, Ink::Dim, Rect::new(0.0, 0.0, w, h), Align::Center);
            return;
        }
        if self.items.is_empty() {
            let mid = h / 2.0;
            cv.text(glyph::CLIPBOARD, Font::IconLarge, Ink::Dim, Rect::new(0.0, mid - 52.0, w, 32.0), Align::Center);
            cv.text("Copy something and it shows up here", Font::Body, Ink::Text, Rect::new(0.0, mid - 14.0, w, 20.0), Align::Center);
            let note = "Kept in memory only · copies from password managers are skipped";
            cv.text(note, Font::Small, Ink::Dim, Rect::new(0.0, mid + 8.0, w, 18.0), Align::Center);
            return;
        }

        let now = Instant::now();
        for (i, r) in self.rows(w, h) {
            let clip = &self.items[i];
            cv.fill_a(r, 8.0, Ink::Track, cv.hover(r));
            let lead = Rect::new(r.x + 6.0, r.y + 4.0, 40.0, r.h - 8.0);
            let text = Rect::new(r.x + 56.0, r.y, r.w - 56.0 - 64.0, r.h);
            let meta = Rect::new(r.right() - 60.0, r.y, 52.0, r.h);
            match &clip.content {
                Content::Text(s) if clipboard::is_link(s) => {
                    cv.text(glyph::LINK, Font::Icon, Ink::Accent, lead, Align::Center);
                    cv.text(s.trim(), Font::Body, Ink::Accent, text, Align::Left);
                }
                Content::Text(s) => {
                    cv.text(glyph::DOCUMENT, Font::Icon, Ink::Dim, lead, Align::Center);
                    cv.text(&clipboard::preview(s, 200), Font::Body, Ink::Text, text, Align::Left);
                }
                Content::Image { thumb, width, height, .. } => {
                    cv.image(thumb, lead, 4.0);
                    cv.text(&format!("Image · {width} × {height}"), Font::Body, Ink::Text, text, Align::Left);
                }
            }
            let copied = self.copied.is_some_and(|(id, _)| id == clip.id);
            let (label, ink) = if copied { ("Copied".into(), Ink::Calm) } else { (age(now - clip.at), Ink::Dim) };
            cv.text(&label, Font::Small, ink, meta, Align::Right);
        }

        button(cv, Self::clear_button(w), glyph::DELETE, Font::Icon, true, false);
        let max = self.items.len().saturating_sub(self.rows_visible.get());
        if max > 0 {
            let track = Rect::new(w - 18.0, 40.0, 3.0, h - 44.0);
            cv.fill(track, 1.5, Ink::Track);
            let thumb_h = track.h * self.rows_visible.get() as f32 / self.items.len() as f32;
            let y = track.y + (track.h - thumb_h) * self.scroll as f32 / max as f32;
            cv.fill(Rect { y, h: thumb_h, ..track }, 1.5, Ink::Dim);
        }
    }

    fn click(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        if !self.items.is_empty() && Self::clear_button(w).contains(x, y) {
            self.items.clear();
            self.scroll = 0;
            return true;
        }
        let Some((i, _)) = self.rows(w, h).find(|(_, r)| r.contains(x, y)) else { return false };
        let Some(watcher) = &self.watcher else { return false };
        if watcher.copy(&self.items[i].content) {
            self.copied = Some((self.items[i].id, Instant::now()));
        }
        true
    }

    fn wheel(&mut self, notches: f32) -> bool {
        let max = self.items.len().saturating_sub(self.rows_visible.get());
        let target = (self.scroll as f32 - notches.round()).clamp(0.0, max as f32) as usize;
        let changed = target != self.scroll;
        self.scroll = target;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::age;
    use std::time::Duration;

    #[test]
    fn ages() {
        assert_eq!(age(Duration::from_secs(30)), "now");
        assert_eq!(age(Duration::from_secs(5 * 60)), "5m");
        assert_eq!(age(Duration::from_secs(2 * 3600 + 10)), "2h");
        assert_eq!(age(Duration::from_secs(3 * 86400)), "3d");
    }
}
