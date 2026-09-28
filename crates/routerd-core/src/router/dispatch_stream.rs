use super::provider_entry::ScopeActiveGuard;
use super::RouterEngine;
use crate::adapters::ByteStream;
use crate::error::{Result, RouterError};
use crate::models::ChatCompletionRequest;
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

        let (req_profile, tier_cfg, thresholds, candidates) = self.prepare_routing(request).await?;
        let ranked = ScoringEngine::rank_candidates(&req_profile, &candidates, &tier_cfg, &thresholds);

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
}
