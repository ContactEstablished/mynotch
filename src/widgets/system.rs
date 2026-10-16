use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::{Compact, Widget};
use crate::gfx::{Align, Canvas, Font, Ink, Rect, glyph};
use crate::stats::{Snapshot, Stats};

const INTERVAL: Duration = Duration::from_secs(1);
/// Seconds of history drawn in the graphs.
const HISTORY: usize = 60;
/// Network graphs scale to their recent peak, but never below this (bytes/s), so an idle
/// connection reads as flat rather than as noise blown up to full height.
const NET_FLOOR: f64 = 64.0 * 1024.0;

/// Live CPU, RAM and network load, with a minute of history.
pub struct SystemWidget {
    stats: Stats,
    /// Newest last; empty until the first sample, since sysinfo needs two CPU refreshes a
    /// while apart before usage is meaningful.
    history: VecDeque<Snapshot>,
    next_sample: Instant,
}

impl SystemWidget {
    pub fn new() -> Self {
        Self { stats: Stats::new(), history: VecDeque::with_capacity(HISTORY), next_sample: Instant::now() + INTERVAL }
    }

    fn latest(&self) -> Option<Snapshot> {
        self.history.back().copied()
    }
}

fn pct(v: Option<f32>) -> String {
    v.map_or("–".into(), |v| format!("{v:.0}%"))
}

/// Human-readable throughput, e.g. "512 B/s", "1.5 KB/s", "150 KB/s", "2.5 MB/s".
fn rate(bytes_per_sec: f64) -> String {
    const UNITS: [&str; 4] = ["B/s", "KB/s", "MB/s", "GB/s"];
    let (mut v, mut unit) = (bytes_per_sec.max(0.0), 0);
    while v >= 1000.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 || v >= 100.0 {
        format!("{v:.0} {}", UNITS[unit])
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

/// Bar graph of `values` (each 0..=1), newest at the right, over a faint track.
fn graph(cv: &Canvas, r: Rect, values: impl ExactSizeIterator<Item = f32>, ink: Ink) {
    cv.fill(r, 6.0, Ink::Track);
    let slot = r.w / HISTORY as f32;
    let first = HISTORY - values.len().min(HISTORY);
    for (i, v) in values.enumerate() {
        let bh = (r.h - 4.0) * v.clamp(0.0, 1.0);
        if bh >= 0.5 {
            let x = r.x + (first + i) as f32 * slot;
            cv.fill_a(Rect::new(x + 0.5, r.y + r.h - 2.0 - bh, (slot - 1.0).max(1.0), bh), 0.0, ink, 0.7);
        }
    }
}

impl Widget for SystemWidget {
    fn title(&self) -> &'static str {
        "System"
    }

    fn icon(&self) -> &'static str {
        glyph::PULSE
    }

    fn update(&mut self, now: Instant) -> bool {
        if now < self.next_sample {
            return false;
        }
        self.next_sample = now + INTERVAL;
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(self.stats.sample());
        true
    }

    fn compact(&self) -> Option<Compact> {
        let s = self.latest();
        let text = format!("CPU {}  RAM {}", pct(s.map(|s| s.cpu)), pct(s.map(|s| s.ram)));
        Some(Compact { priority: 1, text, live: false, progress: None })
    }

    fn draw(&self, cv: &Canvas, w: f32, h: f32) {
        let s = self.latest();
        let row_h = h / 3.0;
        let (gx, gw, vx) = (64.0, w - 64.0 - 196.0, w - 180.0);
        let row = |i: usize| i as f32 * row_h;
        let label = |text: &str, y: f32| cv.caps(text, Font::Caption, Ink::Dim, Rect::new(0.0, y, 56.0, row_h), Align::Left);
        let graph_rect = |y: f32| Rect::new(gx, y + 8.0, gw, row_h - 16.0);

        label("CPU", row(0));
        graph(cv, graph_rect(row(0)), self.history.iter().map(|s| s.cpu / 100.0), Ink::Accent);
        cv.text(&pct(s.map(|s| s.cpu)), Font::Heading, Ink::Text, Rect::new(vx, row(0), 180.0, row_h), Align::Right);

        label("RAM", row(1));
        graph(cv, graph_rect(row(1)), self.history.iter().map(|s| s.ram / 100.0), Ink::Accent);
        cv.text(&pct(s.map(|s| s.ram)), Font::Heading, Ink::Text, Rect::new(vx, row(1), 180.0, row_h), Align::Right);

        label("NET", row(2));
        let peak = self.history.iter().map(|s| s.down).fold(NET_FLOOR, f64::max);
        graph(cv, graph_rect(row(2)), self.history.iter().map(|s| (s.down / peak) as f32), Ink::Calm);
        let (down, up) = s.map_or(("–".into(), "–".into()), |s| (rate(s.down), rate(s.up)));
        let half = Rect::new(vx, row(2), 180.0, row_h / 2.0);
        cv.text(&format!("↓  {down}"), Font::Body, Ink::Text, Rect { y: half.y + 6.0, ..half }, Align::Right);
        cv.text(&format!("↑  {up}"), Font::Small, Ink::Dim, Rect { y: half.y + half.h - 2.0, ..half }, Align::Right);
    }

    fn card_weight(&self) -> Option<f32> {
        Some(1.0)
    }

    fn draw_card(&self, cv: &Canvas, w: f32, h: f32) {
        let s = self.latest();
        let row_h = h / 3.0;
        let rows: [(&str, String, Option<f32>); 3] = [
            ("CPU", pct(s.map(|s| s.cpu)), s.map(|s| s.cpu / 100.0)),
            ("RAM", pct(s.map(|s| s.ram)), s.map(|s| s.ram / 100.0)),
            ("NET", s.map_or("–".into(), |s| format!("↓ {}", rate(s.down))), None),
        ];
        for (i, (name, value, fill)) in rows.into_iter().enumerate() {
            let y = i as f32 * row_h;
            cv.caps(name, Font::Caption, Ink::Dim, Rect::new(0.0, y, 40.0, 22.0), Align::Left);
            cv.text(&value, Font::Title, Ink::Text, Rect::new(40.0, y, w - 40.0, 22.0), Align::Right);
            let bar = Rect::new(0.0, y + 26.0, w, 4.0);
            if let Some(f) = fill {
                cv.fill(bar, 2.0, Ink::Track);
                cv.fill(Rect { w: (bar.w * f).max(bar.h), ..bar }, 2.0, Ink::Accent);
            } else if let Some(s) = s {
                cv.text(&format!("↑ {}", rate(s.up)), Font::Small, Ink::Dim, Rect::new(0.0, y + 22.0, w, 16.0), Align::Right);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::rate;

    #[test]
    fn formats_throughput() {
        assert_eq!(rate(0.0), "0 B/s");
        assert_eq!(rate(512.0), "512 B/s");
        assert_eq!(rate(1536.0), "1.5 KB/s");
        assert_eq!(rate(150.0 * 1024.0), "150 KB/s");
        assert_eq!(rate(2.5 * 1024.0 * 1024.0), "2.5 MB/s");
        assert_eq!(rate(-5.0), "0 B/s");
    }
}
