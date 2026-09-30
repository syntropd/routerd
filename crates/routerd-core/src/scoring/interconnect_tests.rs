use super::{CandidateProvider, RequestProfile, ScoringEngine};
use crate::models::{ProviderModelConfig, ReasoningEffort};
use crate::telemetry::PressureLevel;
use crate::{ThresholdsConfig, TierConfig};

fn make_interconnect_candidate(id: &str, beta: Option<f64>) -> CandidateProvider {
    CandidateProvider {
        provider_id: id.to_string(),
        provider_name: id.to_string(),
        provider_kind: "varlink".to_string(),
        provider_tier: "hard".to_string(),
        provider_weight: 1.0,
        provider_enabled: true,
        is_healthy: true,
        recent_failures: 0,
        model: ProviderModelConfig {
            name: format!("{}-model", id),
            max_context_tokens: 32000,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            avg_latency_ms: 50.0,
            tokens_per_second: 100.0,
            tier: Some("hard".to_string()),
        },
        psi_level: PressureLevel::Normal,
        psi_memory_some: 0.0,
        beta_link: beta,
    }
}

#[test]
fn test_interconnect_penalty_applied_on_high_effort() {
    let nvlink_cand = make_interconnect_candidate("nvlink-node", Some(1.0));
    let pcie_cand = make_interconnect_candidate("pcie-node", Some(0.85));

    let req = RequestProfile {
        requested_model: "router:hard".to_string(),
        requested_tier: "hard".to_string(),
        estimated_prompt_tokens: 500,
        estimated_output_tokens: 1000,
        require_stream: false,
        reasoning_effort: Some(ReasoningEffort::High),
    };

    let tier_cfg = TierConfig::default();
    let thresholds = ThresholdsConfig::default();

    let scored_nvlink = ScoringEngine::score_candidate(&req, &nvlink_cand, &tier_cfg, &thresholds);
    let scored_pcie = ScoringEngine::score_candidate(&req, &pcie_cand, &tier_cfg, &thresholds);

    assert!(scored_nvlink.total_score > scored_pcie.total_score);
    let diff = scored_nvlink.total_score - scored_pcie.total_score;
    // Expected penalty: 250.0 * (1.0 - 0.85) = 37.5
    assert!((diff - 37.5).abs() < 1e-3, "Expected diff 37.5, got {}", diff);
    assert!(scored_pcie.reason.contains("ic_pen=37.5"));
    assert!(scored_nvlink.reason.contains("ic_pen=0.0"));
}

#[test]
fn test_interconnect_penalty_host_bridge_on_max_effort() {
    let nvlink_cand = make_interconnect_candidate("nvlink-node", Some(1.0));
    let host_bridge_cand = make_interconnect_candidate("host-bridge-node", Some(0.50));

    let req = RequestProfile {
        requested_model: "router:hard".to_string(),
        requested_tier: "hard".to_string(),
        estimated_prompt_tokens: 500,
        estimated_output_tokens: 1000,
        require_stream: false,
        reasoning_effort: Some(ReasoningEffort::Max),
    };

    let tier_cfg = TierConfig::default();
    let thresholds = ThresholdsConfig::default();

    let scored_nvlink = ScoringEngine::score_candidate(&req, &nvlink_cand, &tier_cfg, &thresholds);
    let scored_hb = ScoringEngine::score_candidate(&req, &host_bridge_cand, &tier_cfg, &thresholds);

    let diff = scored_nvlink.total_score - scored_hb.total_score;
    // Expected penalty: 250.0 * (1.0 - 0.50) = 125.0
    assert!((diff - 125.0).abs() < 1e-3, "Expected diff 125.0, got {}", diff);
    assert!(scored_hb.reason.contains("ic_pen=125.0"));
}

#[test]
fn test_no_penalty_on_low_or_none_effort() {
    let nvlink_cand = make_interconnect_candidate("nvlink-node", Some(1.0));
    let pcie_cand = make_interconnect_candidate("pcie-node", Some(0.85));

    let req = RequestProfile {
        requested_model: "router:hard".to_string(),
        requested_tier: "hard".to_string(),
        estimated_prompt_tokens: 500,
        estimated_output_tokens: 1000,
        require_stream: false,
        reasoning_effort: Some(ReasoningEffort::Low),
    };

    let tier_cfg = TierConfig::default();
    let thresholds = ThresholdsConfig::default();

    let scored_nvlink = ScoringEngine::score_candidate(&req, &nvlink_cand, &tier_cfg, &thresholds);
    let scored_pcie = ScoringEngine::score_candidate(&req, &pcie_cand, &tier_cfg, &thresholds);

    assert_eq!(scored_nvlink.total_score, scored_pcie.total_score);
    assert!(scored_pcie.reason.contains("ic_pen=0.0"));
}

#[test]
fn test_remote_candidate_no_penalty() {
    let remote_cand = make_interconnect_candidate("openai-cloud", None);

    let req = RequestProfile {
        requested_model: "router:hard".to_string(),
        requested_tier: "hard".to_string(),
        estimated_prompt_tokens: 500,
        estimated_output_tokens: 1000,
        require_stream: false,
        reasoning_effort: Some(ReasoningEffort::Max),
    };

    let tier_cfg = TierConfig::default();
    let thresholds = ThresholdsConfig::default();

    let scored = ScoringEngine::score_candidate(&req, &remote_cand, &tier_cfg, &thresholds);
    assert!(scored.reason.contains("ic_pen=0.0"));
}
