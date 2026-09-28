use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdsConfig {
    #[serde(default = "default_max_latency")]
    pub max_latency_ms: u64,
    #[serde(default = "default_psi_memory_threshold")]
    pub psi_memory_threshold: f32,
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,
    #[serde(default = "default_rss_limit_mb")]
    pub rss_limit_mb: usize,
    #[serde(default = "default_min_tokens_per_second")]
    pub min_tokens_per_second: f64,
}

fn default_max_latency() -> u64 {
    15000
}
fn default_psi_memory_threshold() -> f32 {
    25.0
}
fn default_max_retries() -> usize {
    2
}
fn default_rss_limit_mb() -> usize {
    15
}
fn default_min_tokens_per_second() -> f64 {
    10.0
}

impl Default for ThresholdsConfig {
    fn default() -> Self {
        Self {
            max_latency_ms: default_max_latency(),
            psi_memory_threshold: default_psi_memory_threshold(),
            max_retries: default_max_retries(),
            rss_limit_mb: default_rss_limit_mb(),
            min_tokens_per_second: default_min_tokens_per_second(),
        }
    }
}
