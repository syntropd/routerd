use crate::credentials::resolve_credential;
use crate::error::{Result, RouterError};
use crate::models::ProviderModelConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub const DEFAULT_CONFIG_PATH: &str = "/etc/syntrop/routerd.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_listen_tcp")]
    pub listen_tcp: String,
    #[serde(default = "default_listen_unix")]
    pub listen_unix: String,
    #[serde(default = "default_varlink_socket")]
    pub varlink_socket: String,
    #[serde(default = "default_inferenced_socket")]
    pub inferenced_socket: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_listen_tcp() -> String {
    "127.0.0.1:32768".to_string()
}
fn default_listen_unix() -> String {
    "/run/syntrop/router.sock".to_string()
}
fn default_varlink_socket() -> String {
    "/run/syntrop/io.syntrop.Router1".to_string()
}
fn default_inferenced_socket() -> String {
    "/run/syntrop/io.syntrop.Inference1".to_string()
}
fn default_log_level() -> String {
    "info".to_string()
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            listen_tcp: default_listen_tcp(),
            listen_unix: default_listen_unix(),
            varlink_socket: default_varlink_socket(),
            inferenced_socket: default_inferenced_socket(),
            log_level: default_log_level(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdsConfig {
    #[serde(default = "default_max_latency")]
    pub max_latency_ms: u64,
    #[serde(default = "default_psi_memory_threshold")]
    pub psi_memory_threshold: f32,
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,
    #[serde(default = "default_rss_limit_mb")]
    pub rss_limit_mb: usize,
}

fn default_max_latency() -> u64 {
    15000
}
fn default_psi_memory_threshold() -> f32 {
    25.0
}
fn default_max_retries() -> usize {
    2
}
fn default_rss_limit_mb() -> usize {
    15
}

impl Default for ThresholdsConfig {
    fn default() -> Self {
        Self {
            max_latency_ms: default_max_latency(),
            psi_memory_threshold: default_psi_memory_threshold(),
            max_retries: default_max_retries(),
            rss_limit_mb: default_rss_limit_mb(),
        }
    }
}

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
    true
}
fn default_timeout_ms() -> u64 {
    30000
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouterConfig {
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub thresholds: ThresholdsConfig,
    #[serde(default)]
    pub tiers: HashMap<String, TierConfig>,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
}

impl RouterConfig {
    pub fn load_from_str(s: &str) -> Result<Self> {
        let mut config: RouterConfig = toml::from_str(s)
            .map_err(|e| RouterError::Config(format!("Failed to parse configuration TOML: {}", e)))?;
        config.resolve_all_credentials()?;
        config.ensure_defaults();
        Ok(config)
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let p = path.as_ref();
        let content = fs::read_to_string(p).map_err(|e| {
            RouterError::Config(format!("Failed to read config file '{:?}': {}", p, e))
        })?;
        Self::load_from_str(&content)
    }

    pub fn resolve_all_credentials(&mut self) -> Result<()> {
        for provider in &mut self.providers {
            let resolved = resolve_credential(provider.api_key.as_deref(), &provider.id)?;
            provider.api_key = resolved;
        }
        Ok(())
    }

    fn ensure_defaults(&mut self) {
        if !self.tiers.contains_key("fast") {
            self.tiers.insert(
                "fast".to_string(),
                TierConfig {
                    name: "fast".to_string(),
                    latency_weight: 0.60,
                    cost_weight: 0.30,
                    capability_weight: 0.10,
                    default_model: None,
                },
            );
        }
        if !self.tiers.contains_key("hard") {
            self.tiers.insert(
                "hard".to_string(),
                TierConfig {
                    name: "hard".to_string(),
                    latency_weight: 0.15,
                    cost_weight: 0.15,
                    capability_weight: 0.70,
                    default_model: None,
                },
            );
        }
        for provider in &mut self.providers {
            if provider.id.is_empty() {
                provider.id = if !provider.name.is_empty() {
                    provider.name.clone()
                } else {
                    format!("provider-{}", uuid::Uuid::new_v4())
                };
            }
            if provider.name.is_empty() {
                provider.name = provider.id.clone();
            }
            if provider.kind.is_empty() {
                provider.kind = "openai".to_string();
            }
        }
    }

    pub fn get_tier_config(&self, tier: &str) -> TierConfig {
        self.tiers
            .get(tier)
            .cloned()
            .unwrap_or_else(TierConfig::default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sample_config() {
        let toml_str = r#"
        [daemon]
        listen_tcp = "127.0.0.1:32768"

        [tiers.fast]
        latency_weight = 0.7
        cost_weight = 0.2
        capability_weight = 0.1

        [[providers]]
        id = "test-provider"
        kind = "openai"
        base_url = "https://api.example.com/v1"
        tier = "fast"

        [[providers.models]]
        name = "test-model"
        max_context_tokens = 64000
        "#;

        let cfg = RouterConfig::load_from_str(toml_str).unwrap();
        assert_eq!(cfg.daemon.listen_tcp, "127.0.0.1:32768");
        assert_eq!(cfg.providers.len(), 1);
        assert_eq!(cfg.providers[0].id, "test-provider");
        assert_eq!(cfg.providers[0].models[0].name, "test-model");
        assert_eq!(cfg.providers[0].models[0].max_context_tokens, 64000);
    }
}
