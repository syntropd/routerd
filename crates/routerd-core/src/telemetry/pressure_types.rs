use serde::{Deserialize, Serialize};

pub const DEFAULT_INFERENCED_SOCKET: &str = "/run/syntrop/io.syntrop.Inference1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PressureLevel {
    #[default]
    Normal,
    Elevated,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PressureMetrics {
    pub level: PressureLevel,
    pub memory_some_avg10: f32,
    pub memory_full_avg10: f32,
    pub cpu_some_avg10: f32,
    pub io_some_avg10: f32,
    pub runqueue_latency_us: u64,
    pub ebpf_active: bool,
    pub source: String,
}

impl Default for PressureMetrics {
    fn default() -> Self {
        Self {
            level: PressureLevel::Normal,
            memory_some_avg10: 0.0,
            memory_full_avg10: 0.0,
            cpu_some_avg10: 0.0,
            io_some_avg10: 0.0,
            runqueue_latency_us: 0,
            ebpf_active: false,
            source: "default".to_string(),
        }
    }
}

impl PressureMetrics {
    /// Returns true when memory pressure spikes (some avg10 > 25.0 or full avg10 > 10.0).
    pub fn is_memory_pressure_spike(&self) -> bool {
        self.memory_some_avg10 > 25.0 || self.memory_full_avg10 > 10.0
    }

    /// Returns true when CPU runqueue latency spikes (> 50,000 us or cpu_some > 60.0).
    pub fn is_cpu_contention_spike(&self) -> bool {
        self.runqueue_latency_us > 50_000 || self.cpu_some_avg10 > 60.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_mean_no_pressure() {
        assert_eq!(PressureLevel::default(), PressureLevel::Normal);
        let m = PressureMetrics::default();
        assert_eq!(m.memory_some_avg10, 0.0);
        assert_eq!(m.source, "default");
        assert!(!m.is_memory_pressure_spike());
        assert!(!m.is_cpu_contention_spike());
    }

    #[test]
    fn test_pressure_spike_detection() {
        let m = PressureMetrics {
            memory_some_avg10: 26.0,
            ..Default::default()
        };
        assert!(m.is_memory_pressure_spike());

        let m2 = PressureMetrics {
            runqueue_latency_us: 65_000,
            ..Default::default()
        };
        assert!(m2.is_cpu_contention_spike());
    }
}
