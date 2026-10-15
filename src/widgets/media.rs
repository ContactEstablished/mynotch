use std::time::Instant;

use super::{Compact, Widget};
use crate::gfx::{Align, Canvas, Font, Ink, Rect, glyph};
use crate::media::{Command, Media, NowPlaying};

const ART: f32 = 64.0;
const BUTTON: f32 = 36.0;
const BUTTON_GAP: f32 = 6.0;

/// Now playing (any app that reports to Windows' media controls) with transport buttons.
pub struct MediaWidget {
    media: Media,
    now: Option<NowPlaying>,
}

impl MediaWidget {
    pub fn new() -> Self {
        Self { media: Media::spawn(), now: None }
    }
}

/// Transport buttons, right-aligned and vertically centred: previous, play/pause, next.
fn buttons(w: f32, h: f32) -> [(Command, Rect); 3] {
    let y = (h - BUTTON) / 2.0;
    let x = |i: f32| w - BUTTON - i * (BUTTON + BUTTON_GAP);
    [
        (Command::Previous, Rect::new(x(2.0), y, BUTTON, BUTTON)),
        (Command::TogglePlayPause, Rect::new(x(1.0), y, BUTTON, BUTTON)),
        (Command::Next, Rect::new(x(0.0), y, BUTTON, BUTTON)),
    ]
}

fn enabled(now: &NowPlaying, command: Command) -> bool {
    match command {
        Command::Previous => now.can_previous,
        Command::TogglePlayPause => now.can_toggle,
        Command::Next => now.can_next,
    }
}

impl Widget for MediaWidget {
    fn title(&self) -> &'static str {
        "Media"
    }

    fn update(&mut self, _now: Instant) -> bool {
        match self.media.poll() {
            Some(now) if now != self.now => {
                self.now = now;
                true
            }
            _ => false,
        }
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
        let art = Rect::new(0.0, (h - ART) / 2.0, ART, ART);
        cv.fill(art, 10.0, Ink::Track);
        cv.text(glyph::MUSIC, Font::IconLarge, Ink::Dim, art, Align::Center);

        let buttons = buttons(w, h);
        let tx = art.right() + 14.0;
        let tw = buttons[0].1.x - 12.0 - tx;
        let mid = h / 2.0;

        let Some(now) = &self.now else {
            cv.text("Nothing playing", Font::Title, Ink::Dim, Rect::new(tx, mid - 22.0, tw, 22.0), Align::Left);
            cv.text("Play something in any app", Font::Small, Ink::Dim, Rect::new(tx, mid, tw, 18.0), Align::Left);
            return;
        };

        let title = if now.title.is_empty() { "Unknown title" } else { &now.title };
        cv.text(title, Font::Title, Ink::Text, Rect::new(tx, mid - 26.0, tw, 22.0), Align::Left);
        cv.text(&now.artist, Font::Small, Ink::Dim, Rect::new(tx, mid - 3.0, tw, 18.0), Align::Left);
        cv.text(&now.app, Font::Small, Ink::Dim, Rect::new(tx, mid + 15.0, tw, 18.0), Align::Left);

        for (command, r) in buttons {
            let on = enabled(now, command);
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let hot = on && cv.hovered(r);
            let (icon, font) = match command {
                Command::Previous => (glyph::PREVIOUS, Font::Icon),
                Command::Next => (glyph::NEXT, Font::Icon),
                Command::TogglePlayPause if now.playing => (glyph::PAUSE, Font::IconMedium),
                Command::TogglePlayPause => (glyph::PLAY, Font::IconMedium),
            };
            if command == Command::TogglePlayPause {
                cv.circle(cx, cy, r.w / 2.0, if hot { Ink::Hover } else { Ink::Track }, None);
            } else if hot {
                cv.circle(cx, cy, r.w / 2.0, Ink::Track, None);
            }
            cv.text(icon, font, if on { Ink::Text } else { Ink::Track }, r, Align::Center);
        }
    }

    fn click(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let Some(now) = &mut self.now else { return false };
        let Some((command, _)) = buttons(w, h).into_iter().find(|(_, r)| r.contains(x, y)) else {
            return false;
        };
        if !enabled(now, command) {
            return false;
        }
        self.media.send(command);
        if command == Command::TogglePlayPause {
            // Optimistic; the worker reports the real state right after the command.
            now.playing = !now.playing;
        }
        true
    }
}
