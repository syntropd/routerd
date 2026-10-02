use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

pub const DEFAULT_INFERENCED_SOCKET: &str = "/run/syntrop/io.syntrop.Inference1";
pub const TUNING_JSON_PATH: &str = "/run/syntrop/tuning.json";

/// Dynamic closed-loop tuning parameters reloaded from `/run/syntrop/tuning.json`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DynamicTuning {
    pub memory_some_threshold: f64,
    pub memory_full_threshold: f64,
    pub k_draft_horizon: usize,
    pub max_tokens_clamp: usize,
}

impl Default for DynamicTuning {
    fn default() -> Self {
        Self {
            memory_some_threshold: 25.0,
            memory_full_threshold: 10.0,
            k_draft_horizon: 4,
            max_tokens_clamp: 128,
        }
    }
}

#[derive(Deserialize)]
struct RawTuning {
    #[serde(default = "default_some")]
    memory_some_threshold: f64,
    #[serde(default = "default_full")]
    memory_full_threshold: f64,
    #[serde(default = "default_k")]
    k_draft_horizon: usize,
    #[serde(default = "default_clamp")]
    max_tokens_clamp: usize,
}

fn default_some() -> f64 { 25.0 }
fn default_full() -> f64 { 10.0 }
fn default_k() -> usize { 4 }
fn default_clamp() -> usize { 128 }

impl RawTuning {
    fn into_dynamic(self) -> DynamicTuning {
        DynamicTuning {
            memory_some_threshold: self.memory_some_threshold,
            memory_full_threshold: self.memory_full_threshold,
            k_draft_horizon: self.k_draft_horizon,
            max_tokens_clamp: self.max_tokens_clamp,
        }
    }
}

struct TuningCache {
    last_path: Option<std::path::PathBuf>,
    last_mtime: Option<SystemTime>,
    tuning: DynamicTuning,
}

static TUNING_CACHE: Mutex<Option<TuningCache>> = Mutex::new(None);

/// Poll dynamic tuning configuration from `/run/syntrop/tuning.json`.
pub fn poll_tuning_config() -> DynamicTuning {
    poll_tuning_from_path(Path::new(TUNING_JSON_PATH))
}

/// Poll dynamic tuning configuration from a custom path (allows test isolation).
pub fn poll_tuning_from_path(path: &Path) -> DynamicTuning {
    let mut guard = match TUNING_CACHE.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };

    let current_mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();

    if let Some(ref mut cache) = *guard {
        if cache.last_path.as_deref() == Some(path) && cache.last_mtime == current_mtime {
            return cache.tuning;
        }
        cache.last_path = Some(path.to_path_buf());
        if let Some(mtime) = current_mtime {
            if let Ok(content) = std::fs::read(path) {
                if let Ok(raw) = serde_json::from_slice::<RawTuning>(&content) {
                    cache.last_mtime = Some(mtime);
                    cache.tuning = raw.into_dynamic();
                    return cache.tuning;
                }
            }
            return cache.tuning;
        }
        cache.last_mtime = None;
        cache.tuning = DynamicTuning::default();
        cache.tuning
    } else {
        let mut tuning = DynamicTuning::default();
        let mut loaded_mtime = None;
        if let Some(mtime) = current_mtime {
            if let Ok(content) = std::fs::read(path) {
                if let Ok(raw) = serde_json::from_slice::<RawTuning>(&content) {
                    tuning = raw.into_dynamic();
                    loaded_mtime = Some(mtime);
                }
            }
        }
        *guard = Some(TuningCache {
            last_path: Some(path.to_path_buf()),
            last_mtime: loaded_mtime,
            tuning,
        });
        tuning
    }
}

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
    /// Returns true when memory pressure spikes based on dynamic tuning thresholds.
    pub fn is_memory_pressure_spike(&self) -> bool {
        let tuning = poll_tuning_config();
        (self.memory_some_avg10 as f64) > tuning.memory_some_threshold
            || (self.memory_full_avg10 as f64) > tuning.memory_full_threshold
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

    #[test]
    fn test_poll_tuning_lifecycle() {
        let temp_dir = tempfile::tempdir().unwrap();
        let tuning_file = temp_dir.path().join("tuning.json");

        let default_cfg = poll_tuning_from_path(&tuning_file);
        assert_eq!(default_cfg, DynamicTuning::default());
        assert_eq!(default_cfg.memory_some_threshold, 25.0);
        assert_eq!(default_cfg.memory_full_threshold, 10.0);
        assert_eq!(default_cfg.k_draft_horizon, 4);
        assert_eq!(default_cfg.max_tokens_clamp, 128);

        let custom_json = r#"{
            "policy": "conservative",
            "memory_some_threshold": 15.0,
            "memory_full_threshold": 5.0,
            "k_draft_horizon": 2,
            "max_tokens_clamp": 64
        }"#;
        std::fs::write(&tuning_file, custom_json).unwrap();

        let loaded = poll_tuning_from_path(&tuning_file);
        assert_eq!(loaded.memory_some_threshold, 15.0);
        assert_eq!(loaded.memory_full_threshold, 5.0);
        assert_eq!(loaded.k_draft_horizon, 2);
        assert_eq!(loaded.max_tokens_clamp, 64);

        // Malformed write retains last valid tuning and recovers on valid update
        std::fs::write(&tuning_file, "{ malformed").unwrap();
        let retained = poll_tuning_from_path(&tuning_file);
        assert_eq!(retained, loaded);
        std::fs::write(&tuning_file, custom_json).unwrap();
        let recovered = poll_tuning_from_path(&tuning_file);
        assert_eq!(recovered, loaded);

        std::fs::remove_file(&tuning_file).unwrap();
        let fallback = poll_tuning_from_path(&tuning_file);
        assert_eq!(fallback, DynamicTuning::default());

        // Alternate path isolation check
        let other_file = temp_dir.path().join("other.json");
        assert_eq!(poll_tuning_from_path(&other_file), DynamicTuning::default());
    }
}

