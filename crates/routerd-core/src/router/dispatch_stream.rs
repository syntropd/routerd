use super::provider_entry::ScopeActiveGuard;
use super::RouterEngine;
use crate::adapters::ByteStream;
use crate::error::{Result, RouterError};
use crate::models::ChatCompletionRequest;
use crate::router::cascade::{CascadeDecision, CascadeRouter, ElasticFamilyDowngrader, VramPressureLevel};
use crate::scoring::{ScoredCandidate, ScoringEngine};
use std::sync::atomic::Ordering;
use tracing::{debug, warn};

impl RouterEngine {
    /// Primary entrypoint: routes and dispatches a streaming chat completion request.
    pub async fn route_chat_stream(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<(ByteStream, ScoredCandidate)> {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.active_requests.fetch_add(1, Ordering::SeqCst);
        let _guard = ScopeActiveGuard(&self.active_requests);

        let mut req = request.clone();
        let is_family = req.model.is_empty()
            || req.model == "auto"
            || req.model == "router:auto"
            || req.model.to_ascii_lowercase().contains("7b")
            || req.model == "qwen2.5-7b";

        if is_family {
            let cascade = CascadeRouter::default();
            if let CascadeDecision::System1Cpu { model, .. } = cascade.classify(&req) {
                debug!("CascadeRouter routed streaming query to CPU draft model '{}'", model);
                req.model = model;
            }
        }

        let (psi, hw_report) = tokio::join!(
            self.telemetry.get_pressure(),
            self.hardware_telemetry.get_report(),
        );
        let vram_used: u64 = hw_report.gpus.iter().map(|g| g.used_vram_bytes).sum();
        let vram_total: u64 = hw_report.gpus.iter().map(|g| g.total_vram_bytes).sum();
        let downgrader = ElasticFamilyDowngrader::default();
        let pressure = downgrader.assess_pressure(
            vram_used,
            vram_total,
            psi.memory_some_avg10 as f64,
            psi.memory_full_avg10 as f64,
        );
        if pressure != VramPressureLevel::Normal {
            let decision = downgrader.evaluate(&mut req, pressure);
            debug!("ElasticFamilyDowngrader assessed {:?} streaming pressure: {:?}", pressure, decision);
        }

        if psi.is_memory_pressure_spike() {
            debug!("Dynamic PSI memory spike (some={}, full={}); streaming admission controller clamping tokens", psi.memory_some_avg10, psi.memory_full_avg10);
            if let Some(ref mut max_tok) = req.max_tokens {
                *max_tok = (*max_tok).min(128);
            }
        }

        if psi.is_cpu_contention_spike() {
            debug!("CPU runqueue latency spike ({} us); yielding cooperative streaming time slice", psi.runqueue_latency_us);
            tokio::task::yield_now().await;
        }

        let (mut req_profile, tier_cfg, thresholds, mut candidates) = self.prepare_routing(&req).await?;
        let mut ranked = ScoringEngine::rank_candidates(&req_profile, &candidates, &tier_cfg, &thresholds);

        if ranked.is_empty() && req.model != request.model {
            req = request.clone();
            let (rp, _, _, c) = self.prepare_routing(&req).await?;
            req_profile = rp;
            candidates = c;
            ranked = ScoringEngine::rank_candidates(&req_profile, &candidates, &tier_cfg, &thresholds);
        }

        if ranked.is_empty() {
            let total_tokens = req_profile.total_tokens();
            let mut max_limit = 0;
            let mut context_exceeded = false;
            for c in &candidates {
                if c.provider_enabled {
                    max_limit = max_limit.max(c.model.max_context_tokens);
                    if total_tokens > c.model.max_context_tokens {
                        context_exceeded = true;
                    }
                }
            }
            if context_exceeded && total_tokens > max_limit {
                return Err(RouterError::ContextLimitExceeded {
                    model: req_profile.requested_model,
                    requested_tokens: total_tokens,
                    context_limit: max_limit,
                });
            }

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

        for (attempts, scored) in ranked.into_iter().enumerate() {
            if attempts > max_retries {
                break;
            }

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

            // Interleave 512-token chunked prompt prefill pipelining during stream dispatch
            if req_profile.estimated_prompt_tokens > crate::mesh::PREFILL_CHUNK_SIZE {
                let dispatcher = crate::mesh::PrefillPipelineDispatcher::new(req_profile.estimated_prompt_tokens);
                debug!(
                    "Streaming prompt tokens ({}) exceed PREFILL_CHUNK_SIZE (512); interleaving pipelined prefill dispatcher with in-flight chunk bounds",
                    dispatcher.total_tokens
                );
                tokio::task::yield_now().await;
            }

            match entry.adapter.chat_completion_stream(&scored.model_name, &req).await {
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
}
