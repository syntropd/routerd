use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelItem {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub owned_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelListResponse {
    pub object: String,
    pub data: Vec<ModelItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "ProviderModelConfigHelper")]
pub struct ProviderModelConfig {
    pub name: String,
    pub max_context_tokens: usize,
    pub cost_per_input_token: f64,
    pub cost_per_output_token: f64,
    pub avg_latency_ms: f64,
    pub tokens_per_second: f64,
    pub tier: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ProviderModelConfigHelper {
    Simple(String),
    Detailed {
        name: String,
        #[serde(default = "default_max_context")]
        max_context_tokens: usize,
        #[serde(default)]
        cost_per_input_token: f64,
        #[serde(default)]
        cost_per_output_token: f64,
        #[serde(default = "default_latency")]
        avg_latency_ms: f64,
        #[serde(default = "default_tps")]
        tokens_per_second: f64,
        #[serde(default)]
        tier: Option<String>,
    },
}

impl From<ProviderModelConfigHelper> for ProviderModelConfig {
    fn from(helper: ProviderModelConfigHelper) -> Self {
        match helper {
            ProviderModelConfigHelper::Simple(name) => Self {
                name,
                max_context_tokens: default_max_context(),
                cost_per_input_token: 0.0,
                cost_per_output_token: 0.0,
                avg_latency_ms: default_latency(),
                tokens_per_second: default_tps(),
                tier: None,
            },
            ProviderModelConfigHelper::Detailed {
                name,
                max_context_tokens,
                cost_per_input_token,
                cost_per_output_token,
                avg_latency_ms,
                tokens_per_second,
                tier,
            } => Self {
                name,
                max_context_tokens,
                cost_per_input_token,
                cost_per_output_token,
                avg_latency_ms,
                tokens_per_second,
                tier,
            },
        }
    }
}

fn default_max_context() -> usize {
    32768
}

fn default_latency() -> f64 {
    200.0
}

fn default_tps() -> f64 {
    50.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_name_gets_sane_defaults() {
        let cfg: ProviderModelConfig = serde_json::from_str("\"gemma\"").unwrap();
        assert_eq!(cfg.name, "gemma");
        assert_eq!(cfg.max_context_tokens, 32768);
        assert_eq!(cfg.avg_latency_ms, 200.0);
        assert_eq!(cfg.tokens_per_second, 50.0);
        assert_eq!(cfg.tier, None);
    }

    #[test]
    fn detailed_shape_fills_missing_fields() {
        let cfg: ProviderModelConfig =
            serde_json::from_str(r#"{"name": "qwen", "max_context_tokens": 8192}"#).unwrap();
        assert_eq!(cfg.name, "qwen");
        assert_eq!(cfg.max_context_tokens, 8192);
        assert_eq!(cfg.cost_per_input_token, 0.0);
        assert_eq!(cfg.tokens_per_second, 50.0);
    }
}
