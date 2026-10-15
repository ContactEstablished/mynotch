use std::time::{Duration, Instant};

use super::{Compact, Widget};
use crate::gfx::{Align, Canvas, Font, Ink, Rect};
use crate::stats::{Snapshot, Stats};

const INTERVAL: Duration = Duration::from_secs(1);

/// Live CPU and RAM load.
pub struct SystemWidget {
    stats: Stats,
    /// `None` until the first sample; sysinfo needs two CPU refreshes a while apart
    /// before usage is meaningful.
    snap: Option<Snapshot>,
    next_sample: Instant,
}

impl SystemWidget {
    pub fn new() -> Self {
        Self { stats: Stats::new(), snap: None, next_sample: Instant::now() + INTERVAL }
    }
}

fn pct(v: Option<f32>) -> String {
    v.map_or("–".into(), |v| format!("{v:.0}%"))
}

impl Widget for SystemWidget {
    fn title(&self) -> &'static str {
        "System"
    }

    fn update(&mut self, now: Instant) -> bool {
        if now < self.next_sample {
            return false;
        }
        self.next_sample = now + INTERVAL;
        let s = Some(self.stats.sample());
        let changed = s != self.snap;
        self.snap = s;
        changed
    }

    fn compact(&self) -> Option<Compact> {
        let (cpu, ram) = (self.snap.map(|s| s.cpu), self.snap.map(|s| s.ram));
        Some(Compact { priority: 1, text: format!("CPU {}  RAM {}", pct(cpu), pct(ram)) })
    }

    fn draw(&self, cv: &Canvas, w: f32, _h: f32) {
        let rows = [("CPU", self.snap.map(|s| s.cpu)), ("RAM", self.snap.map(|s| s.ram))];
        for (i, (label, value)) in rows.into_iter().enumerate() {
            let y = 16.0 + i as f32 * 36.0;
            cv.text(label, Font::Small, Ink::Dim, Rect::new(0.0, y, 44.0, 18.0), Align::Left);
            let track = Rect::new(50.0, y + 5.0, w - 110.0, 8.0);
            cv.fill(track, 4.0, Ink::Track);
            if let Some(v) = value {
                cv.fill(Rect { w: (track.w * v / 100.0).max(1.0), ..track }, 4.0, Ink::Accent);
            }
            cv.text(&pct(value), Font::Small, Ink::Text, Rect::new(w - 50.0, y, 50.0, 18.0), Align::Right);
        }
    }
}
