use super::{PressureLevel, PressureMetrics, TelemetryClient};
use std::fs;
use std::path::Path;

impl TelemetryClient {
    /// Read Linux PSI from `/proc/pressure/memory` or check `INFERENCED_SIMULATE_PSI`.
    pub fn read_kernel_or_simulated_psi() -> PressureMetrics {
        if let Ok(sim) = std::env::var("INFERENCED_SIMULATE_PSI") {
            let sim_lower = sim.to_ascii_lowercase();
            if sim_lower == "critical" {
                return PressureMetrics {
                    level: PressureLevel::Critical,
                    memory_some_avg10: 65.0,
                    memory_full_avg10: 30.0,
                    cpu_some_avg10: 20.0,
                    io_some_avg10: 10.0,
                    source: "simulated:critical".to_string(),
                };
            } else if sim_lower == "elevated" {
                return PressureMetrics {
                    level: PressureLevel::Elevated,
                    memory_some_avg10: 25.0,
                    memory_full_avg10: 5.0,
                    cpu_some_avg10: 15.0,
                    io_some_avg10: 5.0,
                    source: "simulated:elevated".to_string(),
                };
            }
        }

        let psi_path = Path::new("/proc/pressure/memory");
        if psi_path.exists() {
            if let Ok(content) = fs::read_to_string(psi_path) {
                let mut mem_some = 0.0f32;
                let mut mem_full = 0.0f32;
                for line in content.lines() {
                    if line.starts_with("some ") {
                        if let Some(avg10) = parse_psi_avg10(line) {
                            mem_some = avg10;
                        }
                    } else if line.starts_with("full ") {
                        if let Some(avg10) = parse_psi_avg10(line) {
                            mem_full = avg10;
                        }
                    }
                }

                let level = if mem_some >= 40.0 || mem_full >= 20.0 {
                    PressureLevel::Critical
                } else if mem_some >= 15.0 || mem_full >= 5.0 {
                    PressureLevel::Elevated
                } else {
                    PressureLevel::Normal
                };

                return PressureMetrics {
                    level,
                    memory_some_avg10: mem_some,
                    memory_full_avg10: mem_full,
                    cpu_some_avg10: 0.0,
                    io_some_avg10: 0.0,
                    source: "kernel:/proc/pressure/memory".to_string(),
                };
            }
        }

        PressureMetrics::default()
    }
}

fn parse_psi_avg10(line: &str) -> Option<f32> {
    for part in line.split_whitespace() {
        if let Some(val_str) = part.strip_prefix("avg10=") {
            return val_str.parse::<f32>().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_psi_parsing() {
        let sample = "some avg10=18.42 avg60=12.11 avg300=5.00 total=1234567\nfull avg10=2.30 avg60=1.10 avg300=0.50 total=45678";
        let mut some_val = 0.0;
        let mut full_val = 0.0;
        for line in sample.lines() {
            if line.starts_with("some ") {
                some_val = parse_psi_avg10(line).unwrap();
            } else if line.starts_with("full ") {
                full_val = parse_psi_avg10(line).unwrap();
            }
        }
        assert_eq!(some_val, 18.42);
        assert_eq!(full_val, 2.30);
    }
}
