use crate::config::TierConfig;
use crate::error::{Result, RouterError};
use crate::models::{ChatCompletionRequest, ProviderModelConfig};
use crate::telemetry::PressureLevel;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestProfile {
    pub requested_model: String,
    pub requested_tier: String,
    pub estimated_prompt_tokens: usize,
    pub estimated_output_tokens: usize,
    pub require_stream: bool,
}

impl RequestProfile {
    pub fn from_request(req: &ChatCompletionRequest, default_tier: &str) -> Self {
        let requested_tier = req
            .requested_tier()
            .map(|t| t.to_string())
            .unwrap_or_else(|| default_tier.to_string());

        let prompt_tokens = req.estimate_prompt_tokens();
        let output_tokens = req
            .max_completion_tokens
            .or(req.max_tokens)
            .unwrap_or(2048);

        Self {
            requested_model: req.model.clone(),
            requested_tier,
            estimated_prompt_tokens: prompt_tokens,
            estimated_output_tokens: output_tokens,
            require_stream: req.stream.unwrap_or(false),
        }
    }

    pub fn total_tokens(&self) -> usize {
        self.estimated_prompt_tokens + self.estimated_output_tokens
    }
}

#[derive(Debug, Clone)]
pub struct CandidateProvider {
    pub provider_id: String,
    pub provider_name: String,
    pub provider_kind: String,
    pub provider_weight: f64,
    pub provider_enabled: bool,
    pub is_healthy: bool,
    pub recent_failures: u32,
    pub model: ProviderModelConfig,
    pub psi_level: PressureLevel,
    pub psi_memory_some: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredCandidate {
    pub provider_id: String,
    pub model_name: String,
    pub total_score: f64,
    pub speed_score: f64,
    pub cost_score: f64,
    pub capability_score: f64,
    pub estimated_cost: f64,
    pub disqualified: bool,
    pub reason: String,
}

pub struct ScoringEngine;

impl ScoringEngine {
    /// Evaluate and score a single candidate for a given request profile and tier configuration.
    pub fn score_candidate(
        req: &RequestProfile,
        cand: &CandidateProvider,
        tier_cfg: &TierConfig,
    ) -> ScoredCandidate {
        if !cand.provider_enabled {
            return ScoredCandidate {
                provider_id: cand.provider_id.clone(),
                model_name: cand.model.name.clone(),
                total_score: f64::NEG_INFINITY,
                speed_score: 0.0,
                cost_score: 0.0,
                capability_score: 0.0,
                estimated_cost: 0.0,
                disqualified: true,
                reason: "Provider disabled in configuration".to_string(),
            };
        }

        // 1. Context limit validation
        let total_tokens = req.total_tokens();
        if total_tokens > cand.model.max_context_tokens {
            return ScoredCandidate {
                provider_id: cand.provider_id.clone(),
                model_name: cand.model.name.clone(),
                total_score: f64::NEG_INFINITY,
                speed_score: 0.0,
                cost_score: 0.0,
                capability_score: 0.0,
                estimated_cost: 0.0,
                disqualified: true,
                reason: format!(
                    "Context limit exceeded (tokens: {}, limit: {})",
                    total_tokens, cand.model.max_context_tokens
                ),
            };
        }

        // If specific model was explicitly requested, check for match or router alias
        let is_router_alias = req.requested_model.starts_with("router:")
            || req.requested_model == "fast"
            || req.requested_model == "hard"
            || req.requested_model == "auto"
            || req.requested_model.is_empty();

        if !is_router_alias && cand.model.name != req.requested_model {
            // Not the model explicitly requested by caller
            return ScoredCandidate {
                provider_id: cand.provider_id.clone(),
                model_name: cand.model.name.clone(),
                total_score: f64::NEG_INFINITY,
                speed_score: 0.0,
                cost_score: 0.0,
                capability_score: 0.0,
                estimated_cost: 0.0,
                disqualified: true,
                reason: format!("Model name does not match requested '{}'", req.requested_model),
            };
        }

        // 2. Cost calculation (0.0 to 100.0)
        let cost = (req.estimated_prompt_tokens as f64 * cand.model.cost_per_input_token)
            + (req.estimated_output_tokens as f64 * cand.model.cost_per_output_token);
        let cost_score = if cost <= 0.0 {
            100.0
        } else {
            100.0 / (1.0 + (cost * 2000.0))
        };

        // 3. Speed calculation (0.0 to 100.0)
        let latency_factor = 100.0 / (1.0 + (cand.model.avg_latency_ms / 150.0));
        let tps_factor = (cand.model.tokens_per_second / 200.0).min(1.0) * 100.0;
        let speed_score = (0.6 * latency_factor + 0.4 * tps_factor).clamp(0.0, 100.0);

        // 4. Capability / Difficulty match (0.0 to 100.0)
        let model_tier = cand
            .model
            .tier
            .as_deref()
            .unwrap_or(if cand.model.max_context_tokens > 64000 { "hard" } else { "fast" });

        let capability_score = if req.requested_tier == "hard" {
            if model_tier == "hard" {
                95.0
            } else {
                35.0
            }
        } else if req.requested_tier == "fast" {
            if model_tier == "fast" {
                95.0
            } else {
                55.0
            }
        } else {
            75.0
        };

        // 5. PSI / Telemetry Penalty
        let mut telemetry_penalty = 0.0f64;
        let is_local = cand.provider_kind == "varlink" || cand.provider_id.contains("local");
        if is_local {
            match cand.psi_level {
                PressureLevel::Critical => {
                    telemetry_penalty = 80.0;
                }
                PressureLevel::Elevated => {
                    telemetry_penalty = 25.0;
                }
                PressureLevel::Normal => {}
            }
            if cand.psi_memory_some > 30.0 {
                telemetry_penalty += cand.psi_memory_some as f64 * 0.5;
            }
        }

        // 6. Health & Recent Failures Penalty
        let mut health_penalty = 0.0f64;
        if !cand.is_healthy {
            health_penalty += 50.0;
        }
        health_penalty += cand.recent_failures as f64 * 15.0;

        // 7. Base Weighted Score
        let raw_score = (tier_cfg.latency_weight * speed_score)
            + (tier_cfg.cost_weight * cost_score)
            + (tier_cfg.capability_weight * capability_score);

        let total_score = (raw_score * cand.provider_weight) - telemetry_penalty - health_penalty;

        let reason = format!(
            "speed={:.1}, cost={:.1}, cap={:.1}, weight={:.2}, psi_pen={:.1}, fail_pen={:.1}",
            speed_score, cost_score, capability_score, cand.provider_weight, telemetry_penalty, health_penalty
        );

        ScoredCandidate {
            provider_id: cand.provider_id.clone(),
            model_name: cand.model.name.clone(),
            total_score,
            speed_score,
            cost_score,
            capability_score,
            estimated_cost: cost,
            disqualified: false,
            reason,
        }
    }

    /// Rank candidates for a request and return sorted candidates descending by score.
    pub fn rank_candidates(
        req: &RequestProfile,
        candidates: &[CandidateProvider],
        tier_cfg: &TierConfig,
    ) -> Vec<ScoredCandidate> {
        let mut scored: Vec<ScoredCandidate> = candidates
            .iter()
            .map(|c| Self::score_candidate(req, c, tier_cfg))
            .filter(|s| !s.disqualified)
            .collect();

        scored.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap_or(std::cmp::Ordering::Equal));
        scored
    }

    /// Select the best candidate or return an error if no valid candidates exist.
    pub fn select_best(
        req: &RequestProfile,
        candidates: &[CandidateProvider],
        tier_cfg: &TierConfig,
    ) -> Result<ScoredCandidate> {
        let ranked = Self::rank_candidates(req, candidates, tier_cfg);
        ranked.into_iter().next().ok_or_else(|| {
            RouterError::NoHealthyProvider(format!(
                "No candidate satisfies request (model='{}', tier='{}', tokens={})",
                req.requested_model,
                req.requested_tier,
                req.total_tokens()
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_candidate(id: &str, tier: &str, cost: f64, latency: f64, max_ctx: usize) -> CandidateProvider {
        CandidateProvider {
            provider_id: id.to_string(),
            provider_name: id.to_string(),
            provider_kind: "openai".to_string(),
            provider_weight: 1.0,
            provider_enabled: true,
            is_healthy: true,
            recent_failures: 0,
            model: ProviderModelConfig {
                name: format!("{}-model", id),
                max_context_tokens: max_ctx,
                cost_per_input_token: cost,
                cost_per_output_token: cost,
                avg_latency_ms: latency,
                tokens_per_second: 100.0,
                tier: Some(tier.to_string()),
            },
            psi_level: PressureLevel::Normal,
            psi_memory_some: 0.0,
        }
    }

    #[test]
    fn test_context_limit_disqualification() {
        let cand = sample_candidate("small", "fast", 0.0, 50.0, 1000);
        let req = RequestProfile {
            requested_model: "router:fast".to_string(),
            requested_tier: "fast".to_string(),
            estimated_prompt_tokens: 800,
            estimated_output_tokens: 400,
            require_stream: false,
        };
        let tier_cfg = TierConfig::default();
        let scored = ScoringEngine::score_candidate(&req, &cand, &tier_cfg);
        assert!(scored.disqualified);
        assert!(scored.reason.contains("Context limit exceeded"));
    }

    #[test]
    fn test_fast_tier_prefers_speed() {
        let fast_cand = sample_candidate("fast-node", "fast", 0.000001, 40.0, 32000);
        let slow_cand = sample_candidate("slow-node", "hard", 0.000005, 500.0, 32000);

        let req = RequestProfile {
            requested_model: "router:fast".to_string(),
            requested_tier: "fast".to_string(),
            estimated_prompt_tokens: 200,
            estimated_output_tokens: 200,
            require_stream: false,
        };
        let tier_cfg = TierConfig {
            name: "fast".to_string(),
            latency_weight: 0.70,
            cost_weight: 0.20,
            capability_weight: 0.10,
            default_model: None,
        };

        let scored_fast = ScoringEngine::score_candidate(&req, &fast_cand, &tier_cfg);
        let scored_slow = ScoringEngine::score_candidate(&req, &slow_cand, &tier_cfg);

        assert!(scored_fast.total_score > scored_slow.total_score);
    }

    #[test]
    fn test_hard_tier_prefers_capability() {
        let fast_cand = sample_candidate("fast-node", "fast", 0.000001, 40.0, 32000);
        let hard_cand = sample_candidate("hard-node", "hard", 0.000002, 300.0, 128000);

        let req = RequestProfile {
            requested_model: "router:hard".to_string(),
            requested_tier: "hard".to_string(),
            estimated_prompt_tokens: 500,
            estimated_output_tokens: 1000,
            require_stream: false,
        };
        let tier_cfg = TierConfig {
            name: "hard".to_string(),
            latency_weight: 0.10,
            cost_weight: 0.10,
            capability_weight: 0.80,
            default_model: None,
        };

        let scored_fast = ScoringEngine::score_candidate(&req, &fast_cand, &tier_cfg);
        let scored_hard = ScoringEngine::score_candidate(&req, &hard_cand, &tier_cfg);

        assert!(scored_hard.total_score > scored_fast.total_score);
    }
}
