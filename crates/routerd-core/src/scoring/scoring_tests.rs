use super::{CandidateProvider, RequestProfile, ScoringEngine};
use crate::error::RouterError;
use crate::models::ProviderModelConfig;
use crate::telemetry::PressureLevel;
use crate::{ThresholdsConfig, TierConfig};

fn sample_candidate(id: &str, tier: &str, cost: f64, latency: f64, max_ctx: usize) -> CandidateProvider {
    CandidateProvider {
        provider_id: id.to_string(),
        provider_name: id.to_string(),
        provider_kind: "openai".to_string(),
        provider_tier: tier.to_string(),
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
    let thresholds = ThresholdsConfig::default();
    let scored = ScoringEngine::score_candidate(&req, &cand, &tier_cfg, &thresholds);
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
    let thresholds = ThresholdsConfig::default();

    let scored_fast = ScoringEngine::score_candidate(&req, &fast_cand, &tier_cfg, &thresholds);
    let scored_slow = ScoringEngine::score_candidate(&req, &slow_cand, &tier_cfg, &thresholds);

    assert!(scored_fast.total_score > scored_slow.total_score);
}

#[test]
fn test_tier_default_model_pins_first() {
    let fast_cand = sample_candidate("fast-node", "fast", 0.000001, 40.0, 32000);
    let slow_cand = sample_candidate("slow-node", "hard", 0.000005, 500.0, 32000);
    let req = RequestProfile {
        requested_model: "router:fast".to_string(),
        requested_tier: "fast".to_string(),
        estimated_prompt_tokens: 200,
        estimated_output_tokens: 200,
        require_stream: false,
    };
    let thresholds = ThresholdsConfig::default();

    // No default: scoring order stands (fast first).
    let plain = TierConfig {
        name: "fast".to_string(),
        latency_weight: 0.70,
        cost_weight: 0.20,
        capability_weight: 0.10,
        default_model: None,
    };
    let ranked = ScoringEngine::rank_candidates(
        &req,
        &[fast_cand.clone(), slow_cand.clone()],
        &plain,
        &thresholds,
    );
    assert_eq!(ranked[0].model_name, "fast-node-model");

    // Eligible default jumps the queue even though it scores lower.
    let pinned = TierConfig {
        default_model: Some("slow-node-model".to_string()),
        ..plain.clone()
    };
    let ranked = ScoringEngine::rank_candidates(
        &req,
        &[fast_cand.clone(), slow_cand.clone()],
        &pinned,
        &thresholds,
    );
    assert_eq!(ranked[0].model_name, "slow-node-model");
    assert!(ranked[0].reason.contains("[tier default]"));

    // Unknown default is ignored; scoring order stands.
    let missing = TierConfig {
        default_model: Some("nope".to_string()),
        ..plain.clone()
    };
    let ranked = ScoringEngine::rank_candidates(
        &req,
        &[fast_cand, slow_cand],
        &missing,
        &thresholds,
    );
    assert_eq!(ranked[0].model_name, "fast-node-model");
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
    let thresholds = ThresholdsConfig::default();

    let scored_fast = ScoringEngine::score_candidate(&req, &fast_cand, &tier_cfg, &thresholds);
    let scored_hard = ScoringEngine::score_candidate(&req, &hard_cand, &tier_cfg, &thresholds);

    assert!(scored_hard.total_score > scored_fast.total_score);
}

#[test]
fn test_min_tokens_per_second_disqualification() {
    let mut slow_cand = sample_candidate("slow-node", "fast", 0.000001, 40.0, 32000);
    slow_cand.model.tokens_per_second = 8.5;

    let mut fast_cand = sample_candidate("fast-node", "fast", 0.000001, 40.0, 32000);
    fast_cand.model.tokens_per_second = 50.0;

    let req = RequestProfile {
        requested_model: "router:fast".to_string(),
        requested_tier: "fast".to_string(),
        estimated_prompt_tokens: 100,
        estimated_output_tokens: 100,
        require_stream: false,
    };
    let tier_cfg = TierConfig::default();
    let thresholds = ThresholdsConfig {
        min_tokens_per_second: 10.0,
        ..Default::default()
    };

    // 1. Slow model (< 10.0 tok/s) is disqualified with expected reason
    let scored_slow = ScoringEngine::score_candidate(&req, &slow_cand, &tier_cfg, &thresholds);
    assert!(scored_slow.disqualified);
    assert!(scored_slow.total_score.is_infinite() && scored_slow.total_score < 0.0);
    assert_eq!(
        scored_slow.reason,
        "Model speed 8.5 tok/s is below minimum threshold 10.0 tok/s"
    );

    // 2. Fast model (>= 10.0 tok/s) is eligible
    let scored_fast = ScoringEngine::score_candidate(&req, &fast_cand, &tier_cfg, &thresholds);
    assert!(!scored_fast.disqualified);
    assert!(scored_fast.total_score > 0.0);

    // 3. rank_candidates excludes disqualified slow model
    let ranked = ScoringEngine::rank_candidates(
        &req,
        &[slow_cand.clone(), fast_cand.clone()],
        &tier_cfg,
        &thresholds,
    );
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].provider_id, "fast-node");

    // 4. When min_tokens_per_second is 0.0 (disabled), slow model is not disqualified
    let disabled_thresholds = ThresholdsConfig {
        min_tokens_per_second: 0.0,
        ..Default::default()
    };
    let scored_slow_allowed =
        ScoringEngine::score_candidate(&req, &slow_cand, &tier_cfg, &disabled_thresholds);
    assert!(!scored_slow_allowed.disqualified);

    // 5. select_best returns NoHealthyProvider error when all candidates are below min_tokens_per_second
    let select_res = ScoringEngine::select_best(
        &req,
        &[slow_cand.clone()],
        &tier_cfg,
        &thresholds,
    );
    match select_res {
        Err(RouterError::NoHealthyProvider(msg)) => {
            assert!(msg.contains("No candidate satisfies request"));
        }
        other => panic!("Expected Err(RouterError::NoHealthyProvider), got: {:?}", other),
    }

    // 6. select_best successfully selects healthy fast candidate when mixed
    let select_mixed = ScoringEngine::select_best(
        &req,
        &[slow_cand.clone(), fast_cand.clone()],
        &tier_cfg,
        &thresholds,
    );
    assert!(select_mixed.is_ok());
    assert_eq!(select_mixed.unwrap().provider_id, "fast-node");
}
