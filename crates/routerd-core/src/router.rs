use crate::adapters::{create_adapter, ByteStream, ProviderAdapter};
use crate::config::{ProviderConfig, RouterConfig, TierConfig};
use crate::error::{Result, RouterError};
use crate::models::{
    ChatCompletionRequest, ChatCompletionResponse, ModelItem, ModelListResponse,
};
use crate::scoring::{CandidateProvider, RequestProfile, ScoredCandidate, ScoringEngine};
use crate::telemetry::TelemetryClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, warn};

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
}

pub struct RouterEngine {
    config: Arc<RwLock<RouterConfig>>,
    providers: Arc<RwLock<HashMap<String, ProviderEntry>>>,
    telemetry: TelemetryClient,
    start_time: Instant,
    total_requests: AtomicU64,
    active_requests: AtomicUsize,
}

impl RouterEngine {
    pub fn new(config: RouterConfig) -> Self {
        let telemetry = TelemetryClient::new(&config.daemon.inferenced_socket);
        let mut entries = HashMap::new();

        for p in &config.providers {
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
            start_time: Instant::now(),
            total_requests: AtomicU64::new(0),
            active_requests: AtomicUsize::new(0),
        }
    }

    pub fn telemetry(&self) -> &TelemetryClient {
        &self.telemetry
    }

    /// Primary entrypoint: routes and dispatches a non-streaming chat completion request.
    /// Employs failover retry logic across ranked candidates.
    pub async fn route_chat(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<(ChatCompletionResponse, ScoredCandidate)> {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.active_requests.fetch_add(1, Ordering::SeqCst);
        let _guard = ScopeActiveGuard(&self.active_requests);

        let (req_profile, tier_cfg, candidates) = self.prepare_routing(request).await?;
        let ranked = ScoringEngine::rank_candidates(&req_profile, &candidates, &tier_cfg);

        if ranked.is_empty() {
            return Err(RouterError::NoHealthyProvider(format!(
                "No eligible provider found for model '{}' in tier '{}'",
                req_profile.requested_model, req_profile.requested_tier
            )));
        }

        let max_retries = {
            let cfg = self.config.read().await;
            cfg.thresholds.max_retries
        };

        let mut last_error = None;
        let mut attempts = 0;

        for scored in ranked {
            if attempts > max_retries {
                break;
            }
            attempts += 1;

            debug!(
                "Routing request to provider '{}' (model '{}', score: {:.2})",
                scored.provider_id, scored.model_name, scored.total_score
            );

            let provider_entry = {
                let p_map = self.providers.read().await;
                p_map.get(&scored.provider_id).cloned()
            };

            let entry = match provider_entry {
                Some(e) => e,
                None => continue,
            };

            let start = Instant::now();
            match entry.adapter.chat_completion(&scored.model_name, request).await {
                Ok(resp) => {
                    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                    let mut st = entry.stats.write().await;
                    st.is_healthy = true;
                    st.consecutive_failures = 0;
                    st.total_requests += 1;
                    st.last_latency_ms = elapsed_ms;

                    return Ok((resp, scored));
                }
                Err(e) => {
                    warn!(
                        "Provider '{}' failed dispatch: {}. Attempting failover.",
                        scored.provider_id, e
                    );
                    let mut st = entry.stats.write().await;
                    st.consecutive_failures += 1;
                    st.total_errors += 1;
                    if st.consecutive_failures >= 3 {
                        st.is_healthy = false;
                    }
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            RouterError::NoHealthyProvider("All candidate providers failed".into())
        }))
    }

    /// Primary entrypoint: routes and dispatches a streaming chat completion request.
    pub async fn route_chat_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<(ByteStream, ScoredCandidate)> {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.active_requests.fetch_add(1, Ordering::SeqCst);
        let _guard = ScopeActiveGuard(&self.active_requests);

        let (req_profile, tier_cfg, candidates) = self.prepare_routing(request).await?;
        let ranked = ScoringEngine::rank_candidates(&req_profile, &candidates, &tier_cfg);

        if ranked.is_empty() {
            return Err(RouterError::NoHealthyProvider(format!(
                "No eligible provider found for model '{}' in tier '{}'",
                req_profile.requested_model, req_profile.requested_tier
            )));
        }

        let max_retries = {
            let cfg = self.config.read().await;
            cfg.thresholds.max_retries
        };

        let mut last_error = None;
        let mut attempts = 0;

        for scored in ranked {
            if attempts > max_retries {
                break;
            }
            attempts += 1;

            debug!(
                "Routing streaming request to provider '{}' (model '{}', score: {:.2})",
                scored.provider_id, scored.model_name, scored.total_score
            );

            let provider_entry = {
                let p_map = self.providers.read().await;
                p_map.get(&scored.provider_id).cloned()
            };

            let entry = match provider_entry {
                Some(e) => e,
                None => continue,
            };

            match entry.adapter.chat_completion_stream(&scored.model_name, request).await {
                Ok(stream) => {
                    let mut st = entry.stats.write().await;
                    st.is_healthy = true;
                    st.consecutive_failures = 0;
                    st.total_requests += 1;

                    return Ok((stream, scored));
                }
                Err(e) => {
                    warn!(
                        "Provider '{}' streaming dispatch failed: {}. Attempting failover.",
                        scored.provider_id, e
                    );
                    let mut st = entry.stats.write().await;
                    st.consecutive_failures += 1;
                    st.total_errors += 1;
                    if st.consecutive_failures >= 3 {
                        st.is_healthy = false;
                    }
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            RouterError::NoHealthyProvider("All candidate providers failed for stream".into())
        }))
    }

    async fn prepare_routing(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<(RequestProfile, TierConfig, Vec<CandidateProvider>)> {
        let cfg = self.config.read().await;
        let req_profile = RequestProfile::from_request(request, "fast");
        let tier_cfg = cfg.get_tier_config(&req_profile.requested_tier);

        let psi = self.telemetry.get_pressure().await;
        let mut candidates = Vec::new();

        let p_map = self.providers.read().await;
        for entry in p_map.values() {
            let stats = entry.stats.read().await.clone();

            for m in &entry.config.models {
                candidates.push(CandidateProvider {
                    provider_id: entry.config.id.clone(),
                    provider_name: entry.config.name.clone(),
                    provider_kind: entry.config.kind.clone(),
                    provider_weight: entry.config.weight,
                    provider_enabled: entry.config.enabled,
                    is_healthy: stats.is_healthy,
                    recent_failures: stats.consecutive_failures,
                    model: m.clone(),
                    psi_level: psi.level,
                    psi_memory_some: psi.memory_some_avg10,
                });
            }
        }

        Ok((req_profile, tier_cfg, candidates))
    }

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
            extra: HashMap::new(),
        };

        let (req_profile, tier_cfg, candidates) = self.prepare_routing(&dummy_request).await?;
        let ranked = ScoringEngine::rank_candidates(&req_profile, &candidates, &tier_cfg);
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

        DaemonStatusInfo {
            status: "active".to_string(),
            version: "0.3.0".to_string(),
            uptime_seconds: self.start_time.elapsed().as_secs(),
            total_requests: self.total_requests.load(Ordering::Relaxed),
            active_requests: self.active_requests.load(Ordering::Relaxed),
            providers_count: p_map.len(),
            healthy_providers_count: healthy_count,
            psi_level: format!("{:?}", psi.level),
            psi_memory_some: psi.memory_some_avg10,
        }
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

/// RAII Guard ensuring active_requests decrement upon task drop.
struct ScopeActiveGuard<'a>(&'a AtomicUsize);

impl<'a> Drop for ScopeActiveGuard<'a> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
