use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

/// Snapshot of system load, as percentages in 0..=100.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Snapshot {
    pub cpu: f32,
    pub ram: f32,
}

pub struct Stats {
    sys: System,
}

impl Stats {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        Self { sys }
    }

    pub fn sample(&mut self) -> Snapshot {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        let total = self.sys.total_memory().max(1) as f32;
        Snapshot {
            cpu: self.sys.global_cpu_usage().clamp(0.0, 100.0),
            ram: (self.sys.used_memory() as f32 / total * 100.0).clamp(0.0, 100.0),
        }
    }
}
