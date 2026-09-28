use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_weight")]
    pub latency_weight: f64,
    #[serde(default = "default_weight")]
    pub cost_weight: f64,
    #[serde(default = "default_weight")]
    pub capability_weight: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

fn default_weight() -> f64 {
    0.33
}

impl Default for TierConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            latency_weight: 0.33,
            cost_weight: 0.33,
            capability_weight: 0.34,
            default_model: None,
        }
    }
}
