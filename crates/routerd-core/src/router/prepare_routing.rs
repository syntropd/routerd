use super::{ProviderEntry, ProviderStats, RouterEngine};
use crate::adapters::create_adapter;
use crate::config::{is_local_provider, RouterConfig, ThresholdsConfig, TierConfig};
use crate::error::Result;
use crate::models::ChatCompletionRequest;
use crate::scoring::{CandidateProvider, RequestProfile};
use crate::telemetry::{HardwareTelemetryClient, TelemetryClient};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::warn;

impl RouterEngine {
    pub fn new(config: RouterConfig) -> Self {
        let telemetry = TelemetryClient::new(&config.daemon.inferenced_socket);
        let hardware_telemetry = HardwareTelemetryClient::new(
            &config.daemon.inferenced_socket,
            &config.daemon.runtimed_socket,
        );
        let mut entries = HashMap::new();

        for p in &config.providers {
            // Local-only mode: external providers never load, even if enabled.
            if !is_local_provider(&p.kind, &p.base_url) {
                warn!(
                    "Provider '{}' refused: external LLM APIs are disabled (local-only mode)",
                    p.id
                );
                continue;
            }
            let adapter = create_adapter(p);
            let stats = ProviderStats {
                is_healthy: true,
                consecutive_failures: 0,
                total_requests: 0,
                total_errors: 0,
                last_latency_ms: p.models.first().map(|m| m.avg_latency_ms).unwrap_or(100.0),
            };
            entries.insert(
                p.id.clone(),
                ProviderEntry {
                    config: p.clone(),
                    adapter,
                    stats: Arc::new(RwLock::new(stats)),
                },
            );
        }

        Self {
            config: Arc::new(RwLock::new(config)),
            providers: Arc::new(RwLock::new(entries)),
            telemetry,
            hardware_telemetry,
            start_time: Instant::now(),
            total_requests: AtomicU64::new(0),
            active_requests: AtomicUsize::new(0),
        }
    }

    pub fn telemetry(&self) -> &TelemetryClient {
        &self.telemetry
    }

    pub fn hardware_telemetry(&self) -> &HardwareTelemetryClient {
        &self.hardware_telemetry
    }

    pub(super) async fn prepare_routing(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<(RequestProfile, TierConfig, ThresholdsConfig, Vec<CandidateProvider>)> {
        let cfg = self.config.read().await;
        let req_profile = RequestProfile::from_request(request, "fast");
        let tier_cfg = cfg.get_tier_config(&req_profile.requested_tier);
        let thresholds = cfg.thresholds.clone();

        let (psi, hw_report) = tokio::join!(
            self.telemetry.get_pressure(),
            self.hardware_telemetry.get_report(),
        );

        let local_beta = if hw_report.gpus.len() > 1 {
            let mut min_beta = 1.0f64;
            let mut found_links = false;
            for g in &hw_report.gpus {
                if let Some(links) = &g.p2p_links {
                    for l in links {
                        found_links = true;
                        let b = match l.link_type.as_str() {
                            "NVLink" => 1.0,
                            "PCIe" => 0.85,
                            _ => 0.50,
                        };
                        if b < min_beta {
                            min_beta = b;
                        }
                    }
                }
            }
            if found_links {
                Some(min_beta)
            } else {
                Some(0.50)
            }
        } else {
            Some(1.0)
        };

        let mut candidates = Vec::new();

        let p_map = self.providers.read().await;
        for entry in p_map.values() {
            // Disabled providers never receive traffic.
            if !entry.config.enabled {
                continue;
            }
            let stats = entry.stats.read().await.clone();
            let is_local = entry.config.kind == "varlink" || entry.config.id.contains("local");
            let beta_link = if is_local { local_beta } else { None };

            for m in &entry.config.models {
                candidates.push(CandidateProvider {
                    provider_id: entry.config.id.clone(),
                    provider_name: entry.config.name.clone(),
                    provider_kind: entry.config.kind.clone(),
                    provider_tier: entry.config.tier.clone(),
                    provider_weight: entry.config.weight,
                    provider_enabled: entry.config.enabled,
                    is_healthy: stats.is_healthy,
                    recent_failures: stats.consecutive_failures,
                    model: m.clone(),
                    psi_level: psi.level,
                    psi_memory_some: psi.memory_some_avg10,
                    beta_link,
                });
            }
        }

        Ok((req_profile, tier_cfg, thresholds, candidates))
    }
}
