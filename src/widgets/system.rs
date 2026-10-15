use std::time::{Duration, Instant};

use super::{Compact, Widget};
use crate::gfx::{Align, Canvas, Font, Ink, Rect};
use crate::stats::{Snapshot, Stats};

const INTERVAL: Duration = Duration::from_secs(1);
const ROW_H: f32 = 18.0;

/// Live CPU, RAM and network load.
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
        Some(Compact { priority: 1, text: format!("CPU {}  RAM {}", pct(cpu), pct(ram)), live: false })
    }

    fn draw(&self, cv: &Canvas, w: f32, _h: f32) {
        let label = |text: &str, y: f32| cv.text(text, Font::Small, Ink::Dim, Rect::new(0.0, y, 44.0, ROW_H), Align::Left);

        let bars = [("CPU", self.snap.map(|s| s.cpu)), ("RAM", self.snap.map(|s| s.ram))];
        for (i, (name, value)) in bars.into_iter().enumerate() {
            let y = 8.0 + i as f32 * 28.0;
            label(name, y);
            let track = Rect::new(50.0, y + 5.0, w - 110.0, 8.0);
            cv.fill(track, 4.0, Ink::Track);
            if let Some(v) = value {
                cv.fill(Rect { w: (track.w * v / 100.0).max(1.0), ..track }, 4.0, Ink::Accent);
            }
            cv.text(&pct(value), Font::Small, Ink::Text, Rect::new(w - 50.0, y, 50.0, ROW_H), Align::Right);
        }

        let y = 64.0;
        label("NET", y);
        let (down, up) = match self.snap {
            Some(s) => (rate(s.down), rate(s.up)),
            None => ("–".into(), "–".into()),
        };
        cv.text(&format!("↓  {down}"), Font::Small, Ink::Text, Rect::new(50.0, y, 130.0, ROW_H), Align::Left);
        cv.text(&format!("↑  {up}"), Font::Small, Ink::Text, Rect::new(180.0, y, 130.0, ROW_H), Align::Left);
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
