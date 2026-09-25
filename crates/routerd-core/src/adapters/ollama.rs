use super::{ByteStream, ProviderAdapter};
use crate::config::ProviderConfig;
use crate::error::{Result, RouterError};
use crate::models::{ChatCompletionRequest, ChatCompletionResponse};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::debug;

pub struct OllamaAdapter {
    id: String,
    base_url: String,
    client: Client,
    configured_models: Vec<String>,
}

impl OllamaAdapter {
    pub fn new(cfg: &ProviderConfig) -> Self {
        let timeout = Duration::from_millis(cfg.timeout_ms);
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_default();

        let base = cfg.base_url.trim_end_matches('/').to_string();
        let configured_models = cfg.models.iter().map(|m| m.name.clone()).collect();

        Self {
            id: cfg.id.clone(),
            base_url: base,
            client,
            configured_models,
        }
    }

    fn chat_endpoint(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url)
    }

    fn tags_endpoint(&self) -> String {
        format!("{}/api/tags", self.base_url)
    }

    fn version_endpoint(&self) -> String {
        format!("{}/api/version", self.base_url)
    }

    fn build_payload(&self, target_model: &str, req: &ChatCompletionRequest, stream: bool) -> Value {
        let mut body = json!({
            "model": target_model,
            "messages": req.messages,
            "stream": stream,
        });

        if let Some(temp) = req.temperature {
            body["temperature"] = json!(temp);
        }
        if let Some(top_p) = req.top_p {
            body["top_p"] = json!(top_p);
        }
        if let Some(max_tokens) = req.max_tokens.or(req.max_completion_tokens) {
            body["max_tokens"] = json!(max_tokens);
        }

        body
    }
}

#[async_trait]
impl ProviderAdapter for OllamaAdapter {
    async fn chat_completion(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse> {
        let url = self.chat_endpoint();
        let payload = self.build_payload(target_model, request, false);

        debug!("Ollama adapter [{}]: POST non-streaming to {}", self.id, url);

        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| RouterError::Http(format!("Ollama LAN node '{}' request failed: {}", self.id, e)))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(RouterError::ProviderUnavailable {
                provider: self.id.clone(),
                reason: format!("Ollama HTTP {} - {}", status, err_text),
            });
        }

        let completion: ChatCompletionResponse = resp.json().await.map_err(|e| {
            RouterError::Http(format!("Failed to parse Ollama JSON reply from '{}': {}", self.id, e))
        })?;

        Ok(completion)
    }

    async fn chat_completion_stream(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ByteStream> {
        let url = self.chat_endpoint();
        let payload = self.build_payload(target_model, request, true);

        debug!("Ollama adapter [{}]: POST streaming to {}", self.id, url);

        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| RouterError::Http(format!("Ollama LAN node '{}' streaming request failed: {}", self.id, e)))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(RouterError::ProviderUnavailable {
                provider: self.id.clone(),
                reason: format!("Ollama streaming HTTP {} - {}", status, err_text),
            });
        }

        let stream = resp
            .bytes_stream()
            .map(|chunk_res| chunk_res.map_err(|e| RouterError::Http(e.to_string())));

        Ok(Box::pin(stream))
    }

    async fn health_check(&self) -> Result<bool> {
        let url = self.version_endpoint();
        let res = self
            .client
            .get(&url)
            .timeout(Duration::from_millis(1500))
            .send()
            .await;

        match res {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let url = self.tags_endpoint();
        let resp = match self
            .client
            .get(&url)
            .timeout(Duration::from_secs(3))
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => return Ok(self.configured_models.clone()),
        };

        if !resp.status().is_success() {
            return Ok(self.configured_models.clone());
        }

        let val: Value = resp.json().await.unwrap_or(Value::Null);
        let mut models = Vec::new();
        if let Some(list) = val.get("models").and_then(|m| m.as_array()) {
            for item in list {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    models.push(name.to_string());
                }
            }
        }

        if models.is_empty() {
            Ok(self.configured_models.clone())
        } else {
            Ok(models)
        }
    }
}
