use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use windows::Win32::UI::HiDpi::GetDpiForSystem;

use super::{Compact, Widget};
use crate::apps::{self, AppEntry, Loaded};
use crate::gfx::{Align, Canvas, Font, Image, Ink, Rect, glyph};
use crate::settings::Settings;

const TILE_W: f32 = 84.0;
const TILE_H: f32 = 88.0;
const ICON: f32 = 40.0;
/// How many suggested apps get pinned on first run.
const SUGGESTED_MAX: usize = 8;

#[derive(Clone, Copy)]
enum Tile<'a> {
    App(&'a AppEntry),
    /// Opens the picker.
    Add,
    /// Closes the picker.
    Done,
}

/// App shortcuts: a grid of pinned apps; "+" opens a picker over every installed app.
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
    /// Content size at the last draw, for wheel handling.
    size: Cell<(f32, f32)>,
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
        }
    }

    fn is_pinned(&self, id: &str) -> bool {
        self.pinned.as_ref().is_some_and(|p| p.iter().any(|x| x == id))
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
        for (tile, r) in self.layout(w, h) {
            if cv.hovered(r) {
                cv.fill(r, 12.0, Ink::Track);
            }
            let icon = Rect::new(r.x + (r.w - ICON) / 2.0, r.y + 10.0, ICON, ICON);
            let label = Rect::new(r.x + 4.0, r.y + 58.0, r.w - 8.0, 18.0);
            match tile {
                Tile::App(app) => {
                    match self.icons.get(&app.id) {
                        Some(img) => cv.image(img, icon, 0.0),
                        None => cv.text(glyph::APPS, Font::IconLarge, Ink::Dim, icon, Align::Center),
                    }
                    cv.text(&app.name, Font::Small, Ink::Text, label, Align::Center);
                    if self.picking && self.is_pinned(&app.id) {
                        cv.circle(icon.right() + 2.0, icon.y + 2.0, 6.0, Ink::Accent, None);
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
            Add,
            Done,
        }
        let hit = self.layout(w, h).into_iter().find(|(_, r)| r.contains(x, y)).map(|(tile, _)| match tile {
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
            Some(Hit::App(id)) => apps::launch(&id),
            Some(toggle @ (Hit::Add | Hit::Done)) => {
                self.picking = matches!(toggle, Hit::Add);
                self.scroll = 0;
            }
            None => return false,
        }
        true
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
