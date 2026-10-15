use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use windows::Win32::UI::HiDpi::GetDpiForSystem;

use super::{Compact, Widget};
use crate::apps::{self, AppEntry, Loaded};
use crate::gfx::{Align, Canvas, Font, Image, Ink, Rect, Rgb, glyph};
use crate::settings::Settings;

const TILE_W: f32 = 84.0;
const TILE_H: f32 = 88.0;
const ICON: f32 = 40.0;
const BADGE_R: f32 = 9.0;
/// How many suggested apps get pinned on first run.
const SUGGESTED_MAX: usize = 8;

/// Windows' close-button red, for the unpin badge under the cursor.
const DANGER: Rgb = Rgb(196, 43, 28);
const BADGE: Rgb = Rgb(58, 58, 68);
/// Dark marks on the accent-blue "pinned" badge.
const ON_ACCENT: Rgb = Rgb(12, 12, 18);

fn icon_rect(tile: Rect) -> Rect {
    Rect::new(tile.x + (tile.w - ICON) / 2.0, tile.y + 10.0, ICON, ICON)
}

/// A badge on the top-right corner of a tile's icon, with a generous hit area.
fn badge_rect(tile: Rect) -> Rect {
    let icon = icon_rect(tile);
    let (cx, cy) = (icon.right() + 2.0, icon.y + 2.0);
    Rect::new(cx - 11.0, cy - 11.0, 22.0, 22.0)
}

fn badge(cv: &Canvas, tile: Rect, mark: &str, fill: Ink, ink: Ink) {
    let r = badge_rect(tile);
    cv.circle(r.x + r.w / 2.0, r.y + r.h / 2.0, BADGE_R, fill, None);
    cv.text(mark, Font::IconSmall, ink, r, Align::Center);
}

#[derive(Clone, Copy)]
enum Tile<'a> {
    App(&'a AppEntry),
    /// Opens the picker.
    Add,
    /// Closes the picker.
    Done,
}

/// A pinned app held down: a launch if it's let go where it was, else a drag to reorder.
struct Grab {
    id: String,
    /// Where it was pressed, and from the tile's corner.
    from: (f32, f32),
    offset: (f32, f32),
    /// The cursor now.
    at: (f32, f32),
    /// Moved past `DRAG_SLOP`: a drag, not a click.
    moving: bool,
}

/// How far a press must move to become a drag.
const DRAG_SLOP: f32 = 6.0;

/// Moves pinned app `id` to slot `to` among the `shown` ones (those installed, in order).
/// Pinned apps that aren't installed keep their places. Returns true if it moved.
fn reorder(pinned: &mut Vec<String>, shown: &[String], id: &str, to: usize) -> bool {
    let Some(from) = shown.iter().position(|s| s == id) else { return false };
    let to = to.min(shown.len() - 1);
    let Some(i) = pinned.iter().position(|p| p == id).filter(|_| to != from) else { return false };
    let moved = pinned.remove(i);
    let target = &shown[to];
    // Before the app it displaces when moving back, after it when moving on.
    let at = pinned.iter().position(|p| p == target).map_or(pinned.len(), |at| if to > from { at + 1 } else { at });
    pinned.insert(at, moved);
    true
}

/// App shortcuts: a grid of pinned apps, each with an unpin badge on hover, that can be
/// dragged into a new order; "+" opens a picker over every installed app, where clicking
/// pins or unpins.
pub struct AppsWidget {
    loader: Option<Receiver<Loaded>>,
    /// Every installed app, sorted by name; empty until the loader reports.
    apps: Vec<AppEntry>,
    icons: HashMap<String, Arc<Image>>,
    pinned: Option<Vec<String>>,
    loaded: bool,
    /// Showing the picker rather than the pinned apps.
    picking: bool,
    /// First visible row.
    scroll: usize,
    /// Content size at the last draw, for wheel and drag handling.
    size: Cell<(f32, f32)>,
    grab: Option<Grab>,
}

impl AppsWidget {
    pub fn new() -> Self {
        Self {
            loader: None,
            apps: Vec::new(),
            icons: HashMap::new(),
            pinned: None,
            loaded: false,
            picking: false,
            scroll: 0,
            size: Cell::new((672.0, 176.0)),
            grab: None,
        }
    }

    fn is_pinned(&self, id: &str) -> bool {
        self.pinned.as_ref().is_some_and(|p| p.iter().any(|x| x == id))
    }

    /// Ids of the pinned apps that are installed, in their order on the grid.
    fn shown(&self) -> Vec<String> {
        let pinned = self.pinned.iter().flatten();
        pinned.filter(|id| self.apps.iter().any(|a| &a.id == *id)).cloned().collect()
    }

    /// The pinned app's slot nearest content point (`x`, `y`), on the visible rows.
    fn slot_at(&self, x: f32, y: f32, w: f32, h: f32, count: usize) -> usize {
        let cols = Self::columns(w);
        let left = (w - cols as f32 * TILE_W) / 2.0;
        let col = (((x - left) / TILE_W).max(0.0) as usize).min(cols - 1);
        let row = ((y / TILE_H).max(0.0) as usize).min(Self::visible_rows(h) - 1);
        ((self.scroll + row) * cols + col).min(count.saturating_sub(1))
    }

    fn draw_tile(&self, cv: &Canvas, tile: Tile, r: Rect, hovered: bool) {
        if hovered {
            cv.fill(r, 12.0, Ink::Track);
        }
        let icon = icon_rect(r);
        let label = Rect::new(r.x + 4.0, r.y + 58.0, r.w - 8.0, 18.0);
        match tile {
            Tile::App(app) => {
                match self.icons.get(&app.id) {
                    Some(img) => cv.image(img, icon, 0.0),
                    None => cv.text(glyph::APPS, Font::IconLarge, Ink::Dim, icon, Align::Center),
                }
                cv.text(&app.name, Font::Small, Ink::Text, label, Align::Center);
                if self.picking && self.is_pinned(&app.id) {
                    badge(cv, r, glyph::CHECK, Ink::Accent, Ink::Rgb(ON_ACCENT));
                } else if !self.picking && hovered {
                    let fill = if cv.hovered(badge_rect(r)) { DANGER } else { BADGE };
                    badge(cv, r, glyph::CLOSE, Ink::Rgb(fill), Ink::Text);
                }
            }
            Tile::Add | Tile::Done => {
                let (mark, text) = if matches!(tile, Tile::Add) { ("+", "Add apps") } else { ("✓", "Done") };
                cv.circle(icon.x + ICON / 2.0, icon.y + ICON / 2.0, ICON / 2.0, Ink::Track, None);
                cv.text(mark, Font::Heading, Ink::Text, icon, Align::Center);
                cv.text(text, Font::Small, Ink::Dim, label, Align::Center);
            }
        }
    }

    fn tiles(&self) -> Vec<Tile<'_>> {
        if self.picking {
            std::iter::once(Tile::Done).chain(self.apps.iter().map(Tile::App)).collect()
        } else {
            let pinned = self.pinned.iter().flatten();
            let apps = pinned.filter_map(|id| self.apps.iter().find(|a| &a.id == id)).map(Tile::App);
            apps.chain(std::iter::once(Tile::Add)).collect()
        }
    }

    fn columns(w: f32) -> usize {
        ((w / TILE_W).floor() as usize).max(1)
    }

    fn visible_rows(h: f32) -> usize {
        ((h / TILE_H).floor() as usize).max(1)
    }

    /// Visible tiles with their rects, honouring the scroll position.
    fn layout(&self, w: f32, h: f32) -> Vec<(Tile<'_>, Rect)> {
        let cols = Self::columns(w);
        let left = (w - cols as f32 * TILE_W) / 2.0;
        let (first, last) = (self.scroll * cols, (self.scroll + Self::visible_rows(h)) * cols);
        self.tiles()
            .into_iter()
            .enumerate()
            .filter(|(i, _)| (first..last).contains(i))
            .map(|(i, tile)| {
                let (row, col) = (i / cols - self.scroll, i % cols);
                (tile, Rect::new(left + col as f32 * TILE_W, row as f32 * TILE_H, TILE_W, TILE_H))
            })
            .collect()
    }

    fn max_scroll(&self, w: f32, h: f32) -> usize {
        self.tiles().len().div_ceil(Self::columns(w)).saturating_sub(Self::visible_rows(h))
    }
}

impl Widget for AppsWidget {
    fn title(&self) -> &'static str {
        "Apps"
    }

    fn icon(&self) -> &'static str {
        glyph::TILES
    }

    fn update(&mut self, _now: Instant) -> bool {
        let Some(loader) = &self.loader else { return false };
        let mut changed = false;
        for msg in loader.try_iter() {
            changed = true;
            match msg {
                Loaded::List(apps) => {
                    if self.pinned.is_none() {
                        self.pinned = Some(apps::suggested(&apps, SUGGESTED_MAX));
                    }
                    self.apps = apps;
                    self.loaded = true;
                }
                Loaded::Icon(id, img) => {
                    self.icons.insert(id, img);
                }
            }
        }
        changed
    }

    fn compact(&self) -> Option<Compact> {
        None
    }

    fn draw(&self, cv: &Canvas, w: f32, h: f32) {
        self.size.set((w, h));
        if !self.loaded {
            cv.text("Loading apps…", Font::Body, Ink::Dim, Rect::new(0.0, 0.0, w, h), Align::Center);
            return;
        }
        let dragging = self.grab.as_ref().filter(|g| g.moving);
        for (tile, r) in self.layout(w, h) {
            match (dragging, tile) {
                // The dragged app's slot stays open beneath it.
                (Some(g), Tile::App(app)) if app.id == g.id => cv.fill(r, 12.0, Ink::Track),
                _ => self.draw_tile(cv, tile, r, dragging.is_none() && cv.hovered(r)),
            }
        }
        // The dragged app follows the cursor, over the rest.
        if let Some(g) = dragging
            && let Some(app) = self.apps.iter().find(|a| a.id == g.id)
        {
            let r = Rect::new(g.at.0 - g.offset.0, g.at.1 - g.offset.1, TILE_W, TILE_H);
            cv.fill(r, 12.0, Ink::Hover);
            self.draw_tile(cv, Tile::App(app), r, false);
        }
        // Scrollbar when the grid overflows.
        let max = self.max_scroll(w, h);
        if max > 0 {
            let rows = (max + Self::visible_rows(h)) as f32;
            let track = Rect::new(w - 3.0, 4.0, 3.0, h - 8.0);
            cv.fill(track, 1.5, Ink::Track);
            let thumb_h = track.h * Self::visible_rows(h) as f32 / rows;
            let y = track.y + (track.h - thumb_h) * self.scroll as f32 / max as f32;
            cv.fill(Rect { y, h: thumb_h, ..track }, 1.5, Ink::Dim);
        }
    }

    fn click(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        enum Hit {
            App(String),
            Unpin(String),
            Add,
            Done,
        }
        let hit = self.layout(w, h).into_iter().find(|(_, r)| r.contains(x, y)).map(|(tile, r)| match tile {
            Tile::App(app) if !self.picking && badge_rect(r).contains(x, y) => Hit::Unpin(app.id.clone()),
            Tile::App(app) => Hit::App(app.id.clone()),
            Tile::Add => Hit::Add,
            Tile::Done => Hit::Done,
        });
        match hit {
            Some(Hit::App(id)) if self.picking => {
                let pinned = self.pinned.get_or_insert_with(Vec::new);
                match pinned.iter().position(|p| *p == id) {
                    Some(i) => {
                        pinned.remove(i);
                    }
                    None => pinned.push(id),
                }
            }
            Some(Hit::Unpin(id)) => {
                if let Some(pinned) = &mut self.pinned {
                    pinned.retain(|p| *p != id);
                }
                // The grid shrank; don't leave it scrolled past its end.
                self.scroll = self.scroll.min(self.max_scroll(w, h));
            }
            Some(toggle @ (Hit::Add | Hit::Done)) => {
                self.picking = matches!(toggle, Hit::Add);
                self.scroll = 0;
            }
            // Pinned apps launch when let go; see `press`.
            Some(Hit::App(_)) | None => return false,
        }
        true
    }

    /// A pinned app (not its unpin badge) is grabbed: a launch on release, unless it gets
    /// dragged to a new place.
    fn press(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        if self.picking || !self.loaded {
            return false;
        }
        self.size.set((w, h));
        let grabbed = self.layout(w, h).into_iter().find_map(|(tile, r)| match tile {
            Tile::App(app) if r.contains(x, y) && !badge_rect(r).contains(x, y) => Some((app.id.clone(), r)),
            _ => None,
        });
        let Some((id, r)) = grabbed else { return false };
        self.grab = Some(Grab { id, from: (x, y), offset: (x - r.x, y - r.y), at: (x, y), moving: false });
        true
    }

    /// Past a few pixels, the grabbed app moves to the slot under the cursor as it goes.
    fn drag(&mut self, x: f32, y: f32) -> bool {
        let shown = self.shown();
        let (w, h) = self.size.get();
        let slot = self.slot_at(x, y, w, h, shown.len());
        let Some(g) = &mut self.grab else { return false };
        if g.at == (x, y) {
            return false;
        }
        g.at = (x, y);
        let (dx, dy) = (x - g.from.0, y - g.from.1);
        g.moving |= dx * dx + dy * dy >= DRAG_SLOP * DRAG_SLOP;
        if !g.moving {
            return false;
        }
        let id = g.id.clone();
        if let Some(pinned) = &mut self.pinned {
            reorder(pinned, &shown, &id, slot);
        }
        true
    }

    fn release(&mut self, _x: f32, _y: f32) {
        if let Some(g) = self.grab.take()
            && !g.moving
        {
            apps::launch(&g.id);
        }
    }

    fn wheel(&mut self, notches: f32) -> bool {
        let (w, h) = self.size.get();
        let target = (self.scroll as f32 - notches.round()).clamp(0.0, self.max_scroll(w, h) as f32) as usize;
        let changed = target != self.scroll;
        self.scroll = target;
        changed
    }

    fn load(&mut self, settings: &Settings) {
        self.pinned = settings.apps.clone();
        let px = (ICON * unsafe { GetDpiForSystem() } as f32 / 96.0).round() as u32;
        self.loader = Some(apps::spawn(px, self.pinned.clone().unwrap_or_default()));
    }

    fn store(&self, settings: &mut Settings) {
        if self.pinned.is_some() {
            settings.apps = self.pinned.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::reorder;

    fn ids(s: &str) -> Vec<String> {
        s.split(' ').map(String::from).collect()
    }

    #[test]
    fn reorders_among_shown_apps_and_keeps_hidden_ones() {
        // "x" is pinned but not installed, so not shown.
        let shown = ids("a b c d");
        let mut pinned = ids("a x b c d");
        assert!(reorder(&mut pinned, &shown, "a", 2), "forwards");
        assert_eq!(pinned, ids("x b c a d"));
        let mut pinned = ids("a x b c d");
        assert!(reorder(&mut pinned, &shown, "d", 0), "backwards");
        assert_eq!(pinned, ids("d a x b c"));
        let mut pinned = ids("a x b c d");
        assert!(reorder(&mut pinned, &shown, "b", 99), "past the end: last");
        assert_eq!(pinned, ids("a x c d b"));
    }

    #[test]
    fn same_slot_or_unknown_app_changes_nothing() {
        let shown = ids("a b c");
        let mut pinned = ids("a b c");
        assert!(!reorder(&mut pinned, &shown, "b", 1));
        assert!(!reorder(&mut pinned, &shown, "zzz", 0));
        assert_eq!(pinned, ids("a b c"));
    }
}
