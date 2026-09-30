use super::{CandidateProvider, RequestProfile, ScoredCandidate};
use crate::config::{ThresholdsConfig, TierConfig};
use crate::telemetry::PressureLevel;

pub struct ScoringEngine;

impl ScoringEngine {
    /// Evaluate and score a single candidate for a given request profile and tier configuration.
    pub fn score_candidate(
        req: &RequestProfile,
        cand: &CandidateProvider,
        tier_cfg: &TierConfig,
        thresholds: &ThresholdsConfig,
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

        // 2. Minimum token/sec threshold disqualification
        if thresholds.min_tokens_per_second > 0.0
            && cand.model.tokens_per_second < thresholds.min_tokens_per_second
        {
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
                    "Model speed {:.1} tok/s is below minimum threshold {:.1} tok/s",
                    cand.model.tokens_per_second, thresholds.min_tokens_per_second
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
            .unwrap_or(if !cand.provider_tier.is_empty() {
                &cand.provider_tier
            } else if cand.model.max_context_tokens > 64000 {
                "hard"
            } else {
                "fast"
            });

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

        // 7. Interconnect Penalty for Deep Reasoning (penalize non-NVLink multi-GPU interconnects)
        let mut interconnect_penalty = 0.0f64;
        if let Some(effort) = req.reasoning_effort {
            if matches!(effort, crate::models::ReasoningEffort::High | crate::models::ReasoningEffort::Max) {
                if let Some(beta) = cand.beta_link {
                    interconnect_penalty = 250.0 * (1.0 - beta.clamp(0.0, 1.0));
                }
            }
        }

        // 8. Base Weighted Score
        let raw_score = (tier_cfg.latency_weight * speed_score)
            + (tier_cfg.cost_weight * cost_score)
            + (tier_cfg.capability_weight * capability_score);

        let total_score = (raw_score * cand.provider_weight)
            - telemetry_penalty
            - health_penalty
            - interconnect_penalty;

        let reason = format!(
            "speed={:.1}, cost={:.1}, cap={:.1}, weight={:.2}, psi_pen={:.1}, fail_pen={:.1}, ic_pen={:.1}",
            speed_score, cost_score, capability_score, cand.provider_weight, telemetry_penalty, health_penalty, interconnect_penalty
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

    /// Convenience helper using default thresholds.
    pub fn score_candidate_default(
        req: &RequestProfile,
        cand: &CandidateProvider,
        tier_cfg: &TierConfig,
    ) -> ScoredCandidate {
        Self::score_candidate(req, cand, tier_cfg, &ThresholdsConfig::default())
    }
}
