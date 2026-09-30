use crate::models::ProviderModelConfig;
use crate::telemetry::PressureLevel;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct CandidateProvider {
    pub provider_id: String,
    pub provider_name: String,
    pub provider_kind: String,
    pub provider_tier: String,
    pub provider_weight: f64,
    pub provider_enabled: bool,
    pub is_healthy: bool,
    pub recent_failures: u32,
    pub model: ProviderModelConfig,
    pub psi_level: PressureLevel,
    pub psi_memory_some: f32,
    pub beta_link: Option<f64>,
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
