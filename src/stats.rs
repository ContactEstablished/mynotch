use std::time::Instant;

use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks, RefreshKind, System};

/// Snapshot of system load. `cpu` and `ram` are percentages in 0..=100; `down` and `up` are
/// network throughput in bytes per second, summed over all non-loopback interfaces.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Snapshot {
    pub cpu: f32,
    pub ram: f32,
    pub down: f64,
    pub up: f64,
}

pub struct Stats {
    sys: System,
    networks: Networks,
    last_sample: Instant,
}

impl Stats {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        Self { sys, networks: Networks::new_with_refreshed_list(), last_sample: Instant::now() }
    }

    pub fn sample(&mut self) -> Snapshot {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.networks.refresh(true);

        let now = Instant::now();
        let secs = now.duration_since(self.last_sample).as_secs_f64().max(0.001);
        self.last_sample = now;
        // `received`/`transmitted` count bytes since the previous refresh.
        let (rx, tx) = self
            .networks
            .list()
            .iter()
            .filter(|(name, _)| !name.contains("Loopback"))
            .fold((0, 0), |(rx, tx), (_, n)| (rx + n.received(), tx + n.transmitted()));

        let total = self.sys.total_memory().max(1) as f32;
        Snapshot {
            cpu: self.sys.global_cpu_usage().clamp(0.0, 100.0),
            ram: (self.sys.used_memory() as f32 / total * 100.0).clamp(0.0, 100.0),
            down: rx as f64 / secs,
            up: tx as f64 / secs,
        }
    }
}
