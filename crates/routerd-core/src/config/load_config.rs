use super::{DaemonConfig, ProviderConfig, ThresholdsConfig, TierConfig};
use crate::credentials::resolve_credential;
use crate::error::{Result, RouterError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub const DEFAULT_CONFIG_PATH: &str = "/etc/syntrop/routerd.toml";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouterConfig {
    #[serde(default, alias = "server")]
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
