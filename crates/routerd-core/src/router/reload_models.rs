//! Action page to rescan local models and sync with modeld inventory.

use super::{ProviderEntry, ProviderStats, RouterEngine};
use crate::adapters::create_adapter;
use crate::config::{is_local_provider, ProviderConfig};
use crate::error::Result;
use crate::models::ProviderModelConfig;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::RwLock;
use tokio::time::timeout;
use tracing::{debug, info};

const GGUF_MODELS_DIR: &str = "/var/lib/models/gguf";
const MODELD_SOCKET: &str = "/run/syntrop/io.syntrop.Model1";
const RPC_TIMEOUT: Duration = Duration::from_millis(500);

impl RouterEngine {
    /// Rescans `/var/lib/models/gguf` and queries `modeld` Varlink `List`,
    /// upserting discovered models into the active local provider.
    pub async fn reload_models(&self) -> Result<usize> {
        let mut model_names = HashSet::new();

        // 1. Rescan /var/lib/models/gguf directory (or overridden via SYNTROP_MODELS_GGUF_DIR)
        let gguf_dir_str = std::env::var("SYNTROP_MODELS_GGUF_DIR")
            .unwrap_or_else(|_| GGUF_MODELS_DIR.to_string());
        let gguf_dir = Path::new(&gguf_dir_str);
        if let Ok(mut dir) = tokio::fs::read_dir(gguf_dir).await {
            while let Ok(Some(entry)) = dir.next_entry().await {
                let path = entry.path();
                if path.extension().and_then(|x| x.to_str()) == Some("gguf") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        model_names.insert(stem.to_string());
                    }
                }
            }
        }

        // 2. Query modeld Varlink List over domain socket (or overridden via SYNTROP_MODELD_SOCKET)
        let modeld_socket_str =
            std::env::var("SYNTROP_MODELD_SOCKET").unwrap_or_else(|_| MODELD_SOCKET.to_string());
        let modeld_sock = Path::new(&modeld_socket_str);
        if modeld_sock.exists() {
            if let Ok(Ok(stream)) = timeout(RPC_TIMEOUT, UnixStream::connect(modeld_sock)).await {
                let (reader, mut writer) = stream.into_split();
                let mut reader = BufReader::new(reader);
                let call = json!({ "method": "io.syntrop.Model1.List", "parameters": {} });
                let mut req_bytes = serde_json::to_vec(&call).unwrap_or_default();
                req_bytes.push(0x00);

                if writer.write_all(&req_bytes).await.is_ok() {
                    let mut buf = Vec::new();
                    if timeout(RPC_TIMEOUT, reader.read_until(0x00, &mut buf))
                        .await
                        .is_ok()
                    {
                        if buf.last() == Some(&0x00) {
                            buf.pop();
                        }
                        if let Ok(reply) = serde_json::from_slice::<Value>(&buf) {
                            if let Some(arr) = reply
                                .get("parameters")
                                .and_then(|p| p.get("models"))
                                .and_then(|m| m.as_array())
                            {
                                for m in arr {
                                    if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                                        model_names.insert(id.to_string());
                                    }
                                    if let Some(name) = m.get("name").and_then(|v| v.as_str()) {
                                        model_names.insert(name.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. Upsert discovered models into local provider
        let mut providers = self.providers.write().await;
        let mut local_entry = providers
            .values_mut()
            .find(|p| is_local_provider(&p.config.kind, &p.config.base_url));

        let count = if let Some(ref mut entry) = local_entry {
            for name in &model_names {
                if !entry.config.models.iter().any(|m| &m.name == name) {
                    debug!("Upserting local model: {}", name);
                    entry.config.models.push(ProviderModelConfig {
                        name: name.clone(),
                        max_context_tokens: 8192,
                        cost_per_input_token: 0.0,
                        cost_per_output_token: 0.0,
                        avg_latency_ms: 50.0,
                        tokens_per_second: 30.0,
                        tier: None,
                        draft_model: None,
                    });
                }
            }
            entry.config.models.len()
        } else {
            // Create default local provider if none exists
            let models: Vec<ProviderModelConfig> = model_names
                .into_iter()
                .map(|name| ProviderModelConfig {
                    name,
                    max_context_tokens: 8192,
                    cost_per_input_token: 0.0,
                    cost_per_output_token: 0.0,
                    avg_latency_ms: 50.0,
                    tokens_per_second: 30.0,
                    tier: None,
                    draft_model: None,
                })
                .collect();

            let cfg = ProviderConfig {
                id: "local".to_string(),
                name: "Local Engine".to_string(),
                kind: "runtimed".to_string(),
                base_url: "/run/syntrop/io.syntrop.Runtime1".to_string(),
                api_key: None,
                tier: "fast".to_string(),
                weight: 1.0,
                enabled: true,
                timeout_ms: 30000,
                models,
            };
            let count = cfg.models.len();
            let adapter = create_adapter(&cfg);
            let stats = ProviderStats {
                is_healthy: true,
                consecutive_failures: 0,
                total_requests: 0,
                total_errors: 0,
                last_latency_ms: 50.0,
            };
            providers.insert(
                "local".to_string(),
                ProviderEntry {
                    config: cfg,
                    adapter,
                    stats: Arc::new(RwLock::new(stats)),
                },
            );
            count
        };

        info!("Local provider models reloaded; total: {}", count);
        Ok(count)
    }
}
