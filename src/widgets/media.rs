use std::time::Instant;

use super::{Compact, Widget, button, button_row, clock};
use crate::gfx::{Align, Canvas, Font, Ink, Rect, glyph};
use crate::media::{Command, Media, NowPlaying};

/// Now playing (any app that reports to Windows' media controls): album art, title, a
/// seekable progress bar and transport buttons.
pub struct MediaWidget {
    media: Media,
    now: Option<NowPlaying>,
    /// Whole seconds of position last drawn, so progress redraws once per second.
    shown_secs: u64,
}

impl MediaWidget {
    pub fn new() -> Self {
        Self { media: Media::spawn(), now: None, shown_secs: u64::MAX }
    }
}

fn enabled(now: &NowPlaying, command: Command) -> bool {
    match command {
        Command::Previous => now.can_previous,
        Command::TogglePlayPause => now.can_toggle,
        Command::Next => now.can_next,
        Command::Seek(_) => now.can_seek,
    }
}

/// Where the full panel puts things, for a `w` x `h` content area.
struct PanelLayout {
    art: Rect,
    text_x: f32,
    bar: Rect,
    /// Generous hit area around the bar for seeking.
    seek: Rect,
    buttons: [(Command, Rect); 3],
}

fn panel_layout(w: f32, h: f32) -> PanelLayout {
    let art = Rect::new(0.0, (h - 160.0) / 2.0, 160.0, 160.0);
    let text_x = art.right() + 24.0;
    let tw = w - text_x;
    let bar = Rect::new(text_x, 96.0, tw, 4.0);
    let [prev, play, next] = button_row(text_x + tw / 2.0, 148.0, [36.0, 44.0, 36.0], 16.0);
    PanelLayout {
        art,
        text_x,
        bar,
        seek: Rect::new(text_x, 86.0, tw, 24.0),
        buttons: [(Command::Previous, prev), (Command::TogglePlayPause, play), (Command::Next, next)],
    }
}

struct CardLayout {
    art: Rect,
    bar: Rect,
    buttons: [(Command, Rect); 3],
}

fn card_layout(w: f32, h: f32) -> CardLayout {
    let [prev, play, next] = button_row(w / 2.0, h - 18.0, [32.0, 36.0, 32.0], 14.0);
    CardLayout {
        art: Rect::new(0.0, 0.0, 64.0, 64.0),
        bar: Rect::new(0.0, 80.0, w, 3.0),
        buttons: [(Command::Previous, prev), (Command::TogglePlayPause, play), (Command::Next, next)],
    }
}

fn draw_art(cv: &Canvas, now: Option<&NowPlaying>, r: Rect, radius: f32, font: Font) {
    match now.and_then(|n| n.art.as_ref()) {
        Some(art) => cv.image(art, r, radius),
        None => {
            cv.fill(r, radius, Ink::Track);
            cv.text(glyph::MUSIC, font, Ink::Dim, r, Align::Center);
        }
    }
}

/// Track bar with the elapsed part filled.
fn draw_progress(cv: &Canvas, now: &NowPlaying, bar: Rect) {
    if now.duration.is_zero() {
        return;
    }
    let done = now.position_now().as_secs_f32() / now.duration.as_secs_f32();
    cv.fill(bar, bar.h / 2.0, Ink::Track);
    cv.fill(Rect { w: (bar.w * done).max(bar.h), ..bar }, bar.h / 2.0, Ink::Accent);
}

fn draw_buttons(cv: &Canvas, now: &NowPlaying, buttons: &[(Command, Rect); 3], big: bool) {
    let play_font = if big { Font::IconLarge } else { Font::IconMedium };
    for &(command, r) in buttons {
        let (icon, font) = match command {
            Command::Previous => (glyph::PREVIOUS, Font::Icon),
            Command::Next => (glyph::NEXT, Font::Icon),
            _ if now.playing => (glyph::PAUSE, play_font),
            _ => (glyph::PLAY, play_font),
        };
        button(cv, r, icon, font, enabled(now, command), command == Command::TogglePlayPause);
    }
}

impl MediaWidget {
    /// Shared click handling: transport buttons, then seeking within `seek` along `bar`.
    fn press(&mut self, x: f32, y: f32, buttons: &[(Command, Rect); 3], bar: Rect, seek: Rect) -> bool {
        let Some(now) = &mut self.now else { return false };
        let command = if let Some(&(c, _)) = buttons.iter().find(|(_, r)| r.contains(x, y)) {
            c
        } else if seek.contains(x, y) && now.can_seek {
            Command::Seek(now.duration.mul_f32(((x - bar.x) / bar.w).clamp(0.0, 1.0)))
        } else {
            return false;
        };
        if !enabled(now, command) {
            return false;
        }
        self.media.send(command);
        // Optimistic; the worker reports the real state right after the command.
        match command {
            Command::TogglePlayPause => {
                now.position = now.position_now();
                now.sampled = Instant::now();
                now.playing = !now.playing;
            }
            Command::Seek(to) => {
                now.position = to;
                now.sampled = Instant::now();
            }
            _ => {}
        }
        true
    }
}

impl Widget for MediaWidget {
    fn title(&self) -> &'static str {
        "Media"
    }

    fn icon(&self) -> &'static str {
        glyph::MUSIC
    }

    fn update(&mut self, _now: Instant) -> bool {
        let mut changed = false;
        if let Some(now) = self.media.poll()
            && now != self.now
        {
            self.now = now;
            changed = true;
        }
        let secs = self.now.as_ref().map_or(0, |n| n.position_now().as_secs());
        if secs != self.shown_secs {
            self.shown_secs = secs;
            changed = true;
        }
        changed
    }

    fn compact(&self) -> Option<Compact> {
        let now = self.now.as_ref().filter(|n| n.playing && !n.title.is_empty())?;
        let text = if now.artist.is_empty() {
            format!("♪  {}", now.title)
        } else {
            format!("♪  {} — {}", now.title, now.artist)
        };
        Some(Compact { priority: 10, text, live: true })
    }

    fn draw(&self, cv: &Canvas, w: f32, h: f32) {
        let l = panel_layout(w, h);
        draw_art(cv, self.now.as_ref(), l.art, 14.0, Font::IconLarge);
        let tw = w - l.text_x;
        let Some(now) = &self.now else {
            cv.text("Nothing playing", Font::Heading, Ink::Dim, Rect::new(l.text_x, 56.0, tw, 28.0), Align::Left);
            cv.text("Play something in any app", Font::Body, Ink::Dim, Rect::new(l.text_x, 86.0, tw, 20.0), Align::Left);
            return;
        };

        let title = if now.title.is_empty() { "Unknown title" } else { &now.title };
        cv.text(title, Font::Heading, Ink::Text, Rect::new(l.text_x, 10.0, tw, 28.0), Align::Left);
        cv.text(&now.artist, Font::Body, Ink::Dim, Rect::new(l.text_x, 40.0, tw, 20.0), Align::Left);
        cv.text(&now.app, Font::Small, Ink::Dim, Rect::new(l.text_x, 60.0, tw, 18.0), Align::Left);

        if !now.duration.is_zero() {
            draw_progress(cv, now, l.bar);
            if now.can_seek && cv.hovered(l.seek) {
                let done = now.position_now().as_secs_f32() / now.duration.as_secs_f32();
                cv.circle(l.bar.x + l.bar.w * done, l.bar.y + l.bar.h / 2.0, 6.0, Ink::Text, None);
            }
            let times = Rect::new(l.text_x, 104.0, tw, 18.0);
            cv.text(&clock(now.position_now()), Font::Small, Ink::Dim, times, Align::Left);
            cv.text(&clock(now.duration), Font::Small, Ink::Dim, times, Align::Right);
        }
        draw_buttons(cv, now, &l.buttons, true);
    }

    fn click(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let l = panel_layout(w, h);
        self.press(x, y, &l.buttons, l.bar, l.seek)
    }

    fn card_weight(&self) -> Option<f32> {
        Some(1.6)
    }

    fn draw_card(&self, cv: &Canvas, w: f32, h: f32) {
        let l = card_layout(w, h);
        draw_art(cv, self.now.as_ref(), l.art, 10.0, Font::IconMedium);
        let (tx, tw) = (l.art.right() + 12.0, w - l.art.right() - 12.0);
        let Some(now) = &self.now else {
            cv.text("Nothing playing", Font::Title, Ink::Dim, Rect::new(tx, 20.0, tw, 22.0), Align::Left);
            return;
        };
        let title = if now.title.is_empty() { "Unknown title" } else { &now.title };
        cv.text(title, Font::Title, Ink::Text, Rect::new(tx, 4.0, tw, 22.0), Align::Left);
        cv.text(&now.artist, Font::Small, Ink::Dim, Rect::new(tx, 26.0, tw, 18.0), Align::Left);
        cv.text(&now.app, Font::Small, Ink::Dim, Rect::new(tx, 44.0, tw, 16.0), Align::Left);
        draw_progress(cv, now, l.bar);
        draw_buttons(cv, now, &l.buttons, false);
    }

    fn click_card(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let l = card_layout(w, h);
        let seek = Rect { y: l.bar.y - 8.0, h: 19.0, ..l.bar };
        self.press(x, y, &l.buttons, l.bar, seek)
    }
}
