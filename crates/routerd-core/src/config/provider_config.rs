use crate::models::ProviderModelConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_kind_str")]
    pub kind: String,
    pub base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default = "default_tier_str")]
    pub tier: String,
    #[serde(default = "default_provider_weight")]
    pub weight: f64,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub models: Vec<ProviderModelConfig>,
}

fn default_kind_str() -> String {
    "openai".to_string()
}

fn default_tier_str() -> String {
    "fast".to_string()
}
fn default_provider_weight() -> f64 {
    1.0
}
fn default_enabled() -> bool {
    // Providers stay off unless explicitly enabled (setup only enables
    // entries it verifies live).
    false
}
fn default_timeout_ms() -> u64 {
    30000
}
