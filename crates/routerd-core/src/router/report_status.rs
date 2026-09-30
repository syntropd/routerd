use super::{read_rss_info, DaemonStatusInfo, ProviderStatusInfo, RouterEngine};
use crate::error::{Result, RouterError};
use crate::models::{ChatCompletionRequest, ModelItem, ModelListResponse};
use crate::scoring::{ScoredCandidate, ScoringEngine};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Instant;

impl RouterEngine {
    /// Simulate route evaluation for routerctl / Varlink RouteRequest.
    pub async fn simulate_route(
        &self,
        model: Option<&str>,
        tier: Option<&str>,
        estimated_tokens: Option<usize>,
        require_stream: bool,
    ) -> Result<Vec<ScoredCandidate>> {
        let dummy_request = ChatCompletionRequest {
            model: model.unwrap_or("fast").to_string(),
            messages: vec![],
            temperature: None,
            top_p: None,
            max_tokens: Some(estimated_tokens.unwrap_or(1024)),
            max_completion_tokens: None,
            stream: Some(require_stream),
            tier: tier.map(|t| t.to_string()),
            reasoning_budget: None,
            max_thinking_tokens: None,
            reasoning_content: None,
            reasoning_effort: None,
            extra: HashMap::new(),
        };

        let (req_profile, tier_cfg, thresholds, candidates) = self.prepare_routing(&dummy_request).await?;
        let ranked = ScoringEngine::rank_candidates(&req_profile, &candidates, &tier_cfg, &thresholds);
        Ok(ranked)
    }

    /// Aggregates all model names available across all providers + router virtual models.
    pub async fn list_all_models(&self) -> ModelListResponse {
        let mut model_items = Vec::new();
        let now = self.start_time.elapsed().as_secs();

        // Virtual router models
        for virt in &["router:fast", "router:hard", "router:auto", "fast", "hard"] {
            model_items.push(ModelItem {
                id: virt.to_string(),
                object: "model".to_string(),
                created: now,
                owned_by: "syntrop-routerd".to_string(),
            });
        }

        let p_map = self.providers.read().await;
        for entry in p_map.values() {
            // Disabled providers are configured but never listed or routed to.
            if !entry.config.enabled {
                continue;
            }
            for m in &entry.config.models {
                model_items.push(ModelItem {
                    id: m.name.clone(),
                    object: "model".to_string(),
                    created: now,
                    owned_by: entry.config.id.clone(),
                });
            }
        }

        ModelListResponse {
            object: "list".to_string(),
            data: model_items,
        }
    }

    /// Returns detailed provider status info for CLI and Varlink.
    pub async fn list_provider_statuses(&self) -> Vec<ProviderStatusInfo> {
        let p_map = self.providers.read().await;
        let mut out = Vec::new();

        for entry in p_map.values() {
            let stats = entry.stats.read().await.clone();
            let models = entry.config.models.iter().map(|m| m.name.clone()).collect();
            out.push(ProviderStatusInfo {
                id: entry.config.id.clone(),
                name: entry.config.name.clone(),
                kind: entry.config.kind.clone(),
                base_url: entry.config.base_url.clone(),
                tier: entry.config.tier.clone(),
                is_healthy: stats.is_healthy,
                weight: entry.config.weight,
                models,
                total_requests: stats.total_requests,
                total_errors: stats.total_errors,
                last_latency_ms: stats.last_latency_ms,
            });
        }

        out
    }

    /// Returns high-level daemon status.
    pub async fn get_daemon_status(&self) -> DaemonStatusInfo {
        let psi = self.telemetry.get_pressure().await;
        let p_map = self.providers.read().await;
        let mut healthy_count = 0;

        for entry in p_map.values() {
            let st = entry.stats.read().await;
            if st.is_healthy {
                healthy_count += 1;
            }
        }

        let (rss_bytes, rss_mb) = read_rss_info();

        DaemonStatusInfo {
            status: "active".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_seconds: self.start_time.elapsed().as_secs(),
            total_requests: self.total_requests.load(Ordering::Relaxed),
            active_requests: self.active_requests.load(Ordering::Relaxed),
            providers_count: p_map.len(),
            healthy_providers_count: healthy_count,
            psi_level: format!("{:?}", psi.level),
            psi_memory_some: psi.memory_some_avg10,
            rss_bytes,
            rss_mb,
        }
    }

    /// Returns consolidated hardware and resource telemetry report.
    pub async fn get_hardware_telemetry(&self) -> crate::telemetry::HardwareTelemetryReport {
        self.hardware_telemetry.get_report().await
    }

    /// Test a specific provider by sending a health ping or probe.
    pub async fn test_provider(
        &self,
        provider_id: &str,
    ) -> Result<(bool, f64, Option<String>)> {
        let entry = {
            let p_map = self.providers.read().await;
            p_map
                .get(provider_id)
                .cloned()
                .ok_or_else(|| RouterError::ProviderUnavailable {
                    provider: provider_id.to_string(),
                    reason: "Provider ID not configured".to_string(),
                })?
        };

        let start = Instant::now();
        match entry.adapter.health_check().await {
            Ok(healthy) => {
                let latency = start.elapsed().as_secs_f64() * 1000.0;
                let mut st = entry.stats.write().await;
                st.is_healthy = healthy;
                st.last_latency_ms = latency;
                Ok((healthy, latency, None))
            }
            Err(e) => {
                let latency = start.elapsed().as_secs_f64() * 1000.0;
                let mut st = entry.stats.write().await;
                st.is_healthy = false;
                st.last_latency_ms = latency;
                Ok((false, latency, Some(e.to_string())))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RouterConfig;

    fn empty_engine() -> RouterEngine {
        RouterEngine::new(RouterConfig::default())
    }

    #[tokio::test]
    async fn simulate_route_ranks_nothing_without_providers() {
        let ranked = empty_engine().simulate_route(Some("router:fast"), None, Some(64), false).await.unwrap();
        assert!(ranked.is_empty());
    }

    #[tokio::test]
    async fn list_all_models_always_has_virtual_aliases() {
        let models = empty_engine().list_all_models().await;
        let ids: Vec<&str> = models.data.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"router:fast"));
        assert!(ids.contains(&"router:auto"));
        assert!(empty_engine().list_provider_statuses().await.is_empty());
    }

    #[tokio::test]
    async fn daemon_status_reports_zero_providers() {
        let status = empty_engine().get_daemon_status().await;
        assert_eq!(status.status, "active");
        assert_eq!(status.providers_count, 0);
        assert_eq!(status.healthy_providers_count, 0);
    }

    #[tokio::test]
    async fn test_provider_rejects_unknown_id() {
        let err = empty_engine().test_provider("nope").await.unwrap_err();
        assert!(matches!(err, RouterError::ProviderUnavailable { .. }));
    }
}
