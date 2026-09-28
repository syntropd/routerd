use serde::{Deserialize, Serialize};

pub const DEFAULT_INFERENCED_SOCKET: &str = "/run/syntrop/io.syntrop.Inference1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PressureLevel {
    Normal,
    Elevated,
    Critical,
}

impl Default for PressureLevel {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PressureMetrics {
    pub level: PressureLevel,
    pub memory_some_avg10: f32,
    pub memory_full_avg10: f32,
    pub cpu_some_avg10: f32,
    pub io_some_avg10: f32,
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
            source: "default".to_string(),
        }
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
    }
}
