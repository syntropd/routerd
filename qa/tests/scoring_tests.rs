use routerd_core::{
    CandidateProvider, PressureLevel, ProviderModelConfig, RequestProfile, ScoringEngine,
    TierConfig,
};

fn make_candidate(
    id: &str,
    tier: &str,
    input_cost: f64,
    latency_ms: f64,
    tps: f64,
    context: usize,
) -> CandidateProvider {
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
            max_context_tokens: context,
            cost_per_input_token: input_cost,
            cost_per_output_token: input_cost * 2.0,
            avg_latency_ms: latency_ms,
            tokens_per_second: tps,
            tier: Some(tier.to_string()),
        },
        psi_level: PressureLevel::Normal,
        psi_memory_some: 0.0,
    }
}

#[test]
fn test_context_limit_enforcement() {
    let cand = make_candidate("limited", "fast", 0.0, 50.0, 100.0, 4096);
    let req = RequestProfile {
        requested_model: "router:fast".to_string(),
        requested_tier: "fast".to_string(),
        estimated_prompt_tokens: 3000,
        estimated_output_tokens: 2000, // 5000 total > 4096 limit
        require_stream: false,
    };
    let tier_cfg = TierConfig::default();
    let scored = ScoringEngine::score_candidate(&req, &cand, &tier_cfg);
    assert!(scored.disqualified);
    assert!(scored.total_score.is_infinite() && scored.total_score < 0.0);
}

#[test]
fn test_fast_tier_scoring() {
    let fast_lpu = make_candidate("groq-fast", "fast", 0.0000005, 40.0, 300.0, 32768);
    let slow_heavy = make_candidate("cloud-slow", "hard", 0.000005, 600.0, 40.0, 128000);

    let req = RequestProfile {
        requested_model: "router:fast".to_string(),
        requested_tier: "fast".to_string(),
        estimated_prompt_tokens: 500,
        estimated_output_tokens: 500,
        require_stream: false,
    };

    let tier_cfg = TierConfig {
        name: "fast".to_string(),
        latency_weight: 0.65,
        cost_weight: 0.25,
        capability_weight: 0.10,
        default_model: None,
    };

    let s1 = ScoringEngine::score_candidate(&req, &fast_lpu, &tier_cfg);
    let s2 = ScoringEngine::score_candidate(&req, &slow_heavy, &tier_cfg);

    assert!(!s1.disqualified);
    assert!(!s2.disqualified);
    assert!(
        s1.total_score > s2.total_score,
        "Fast tier should prefer fast_lpu over slow_heavy"
    );
}

#[test]
fn test_hard_tier_scoring() {
    let fast_lpu = make_candidate("groq-fast", "fast", 0.0000005, 40.0, 300.0, 32768);
    let hard_model = make_candidate("reasoner", "hard", 0.000002, 350.0, 60.0, 128000);

    let req = RequestProfile {
        requested_model: "router:hard".to_string(),
        requested_tier: "hard".to_string(),
        estimated_prompt_tokens: 1000,
        estimated_output_tokens: 2000,
        require_stream: false,
    };

    let tier_cfg = TierConfig {
        name: "hard".to_string(),
        latency_weight: 0.10,
        cost_weight: 0.10,
        capability_weight: 0.80,
        default_model: None,
    };

    let s1 = ScoringEngine::score_candidate(&req, &fast_lpu, &tier_cfg);
    let s2 = ScoringEngine::score_candidate(&req, &hard_model, &tier_cfg);

    assert!(
        s2.total_score > s1.total_score,
        "Hard tier should prefer reasoner over fast_lpu"
    );
}

#[test]
fn test_psi_telemetry_penalty_on_local() {
    let mut local_cand = make_candidate("local-syntrop", "fast", 0.0, 30.0, 200.0, 16384);
    local_cand.provider_kind = "varlink".to_string();

    let remote_cand = make_candidate("remote-ollama", "fast", 0.0, 80.0, 100.0, 16384);

    let req = RequestProfile {
        requested_model: "router:fast".to_string(),
        requested_tier: "fast".to_string(),
        estimated_prompt_tokens: 200,
        estimated_output_tokens: 200,
        require_stream: false,
    };
    let tier_cfg = TierConfig::default();

    // 1. Under normal PSI, local wins
    let s_local_norm = ScoringEngine::score_candidate(&req, &local_cand, &tier_cfg);
    let s_remote = ScoringEngine::score_candidate(&req, &remote_cand, &tier_cfg);
    assert!(s_local_norm.total_score > s_remote.total_score);

    // 2. Under critical PSI, local is heavily penalized
    local_cand.psi_level = PressureLevel::Critical;
    local_cand.psi_memory_some = 45.0;
    let s_local_crit = ScoringEngine::score_candidate(&req, &local_cand, &tier_cfg);
    assert!(
        s_remote.total_score > s_local_crit.total_score,
        "Critical PSI should cause remote node to win over local"
    );
}
