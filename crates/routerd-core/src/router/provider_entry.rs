use crate::adapters::ProviderAdapter;
use crate::config::{ProviderConfig, RouterConfig};
use crate::telemetry::{HardwareTelemetryClient, TelemetryClient};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderStats {
    pub is_healthy: bool,
    pub consecutive_failures: u32,
    pub total_requests: u64,
    pub total_errors: u64,
    pub last_latency_ms: f64,
}

#[derive(Clone)]
pub struct ProviderEntry {
    pub config: ProviderConfig,
    pub adapter: Arc<dyn ProviderAdapter>,
    pub stats: Arc<RwLock<ProviderStats>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStatusInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub tier: String,
    pub is_healthy: bool,
    pub weight: f64,
    pub models: Vec<String>,
    pub total_requests: u64,
    pub total_errors: u64,
    pub last_latency_ms: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatusInfo {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub total_requests: u64,
    pub active_requests: usize,
    pub providers_count: usize,
    pub healthy_providers_count: usize,
    pub psi_level: String,
    pub psi_memory_some: f32,
    pub rss_bytes: u64,
    pub rss_mb: f64,
}
pub fn read_rss_info() -> (u64, f64) {
    if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
        let ps = rustix::param::page_size() as u64;
        let mut parts = statm.split_whitespace();
        let _size = parts.next();
        let resident_pages: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let rss_bytes = resident_pages * ps;
        let rss_mb = rss_bytes as f64 / (1024.0 * 1024.0);
        (rss_bytes, rss_mb)
    } else {
        (0, 0.0)
    }
}

pub struct RouterEngine {
    pub(super) config: Arc<RwLock<RouterConfig>>,
    pub(super) providers: Arc<RwLock<HashMap<String, ProviderEntry>>>,
    pub(super) telemetry: TelemetryClient,
    pub(super) hardware_telemetry: HardwareTelemetryClient,
    pub(super) start_time: Instant,
    pub(super) total_requests: AtomicU64,
    pub(super) active_requests: AtomicUsize,
}

/// RAII Guard ensuring active_requests decrement upon task drop.
pub(super) struct ScopeActiveGuard<'a>(pub(super) &'a AtomicUsize);

impl<'a> Drop for ScopeActiveGuard<'a> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
