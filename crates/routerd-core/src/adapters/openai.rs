use super::{ByteStream, ProviderAdapter};
use crate::config::ProviderConfig;
use crate::error::{Result, RouterError};
use crate::models::{ChatCompletionRequest, ChatCompletionResponse};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::debug;

pub struct OpenAICompatibleAdapter {
    id: String,
    base_url: String,
    api_key: Option<String>,
    client: Client,
    configured_models: Vec<String>,
}

impl OpenAICompatibleAdapter {
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
            api_key: cfg.api_key.clone(),
            client,
            configured_models,
        }
    }

    fn chat_endpoint(&self) -> String {
        if self.base_url.ends_with("/chat/completions") {
            self.base_url.clone()
        } else if self.base_url.ends_with("/v1")
            || self.base_url.ends_with("/openai")
            || self.base_url.ends_with("/v1beta")
            || self.base_url.ends_with("/v1alpha")
        {
            format!("{}/chat/completions", self.base_url)
        } else {
            format!("{}/v1/chat/completions", self.base_url)
        }
    }

    fn models_endpoint(&self) -> String {
        if self.base_url.ends_with("/models") {
            self.base_url.clone()
        } else if self.base_url.ends_with("/v1")
            || self.base_url.ends_with("/openai")
            || self.base_url.ends_with("/v1beta")
            || self.base_url.ends_with("/v1alpha")
        {
            format!("{}/models", self.base_url)
        } else {
            format!("{}/v1/models", self.base_url)
        }
    }

    fn build_headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(key) = &self.api_key {
            let auth_val = format!("Bearer {}", key.trim());
            let header_val = HeaderValue::from_str(&auth_val).map_err(|e| {
                RouterError::Credential(format!("Invalid header character in API key: {}", e))
            })?;
            headers.insert(AUTHORIZATION, header_val);
        }

        Ok(headers)
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
        if let Some(max_tokens) = req.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }
        if let Some(max_completion) = req.max_completion_tokens {
            body["max_completion_tokens"] = json!(max_completion);
        }

        for (k, v) in &req.extra {
            if k != "model" && k != "messages" && k != "stream" && k != "tier" {
                body[k] = v.clone();
            }
        }

        body
    }
}

#[async_trait]
impl ProviderAdapter for OpenAICompatibleAdapter {
    async fn chat_completion(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse> {
        let url = self.chat_endpoint();
        let headers = self.build_headers()?;
        let payload = self.build_payload(target_model, request, false);

        debug!("OpenAI adapter [{}]: POST non-streaming to {}", self.id, url);

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .map_err(|e| RouterError::Http(format!("Request to '{}' failed: {}", self.id, e)))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(RouterError::ProviderUnavailable {
                provider: self.id.clone(),
                reason: format!("HTTP {} - {}", status, err_text),
            });
        }

        let completion: ChatCompletionResponse = resp.json().await.map_err(|e| {
            RouterError::Http(format!("Failed to parse JSON reply from '{}': {}", self.id, e))
        })?;

        // Ephemeral lifetime: request and response payloads are owned and dropped
        Ok(completion)
    }

    async fn chat_completion_stream(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ByteStream> {
        let url = self.chat_endpoint();
        let headers = self.build_headers()?;
        let payload = self.build_payload(target_model, request, true);

        debug!("OpenAI adapter [{}]: POST streaming to {}", self.id, url);

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .map_err(|e| RouterError::Http(format!("Streaming request to '{}' failed: {}", self.id, e)))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(RouterError::ProviderUnavailable {
                provider: self.id.clone(),
                reason: format!("Streaming HTTP {} - {}", status, err_text),
            });
        }

        let stream = resp
            .bytes_stream()
            .map(|chunk_res| chunk_res.map_err(|e| RouterError::Http(e.to_string())));

        Ok(Box::pin(stream))
    }

    async fn health_check(&self) -> Result<bool> {
        let url = self.models_endpoint();
        let headers = self.build_headers()?;

        let res = self
            .client
            .get(&url)
            .headers(headers)
            .timeout(Duration::from_secs(5))
            .send()
            .await;

        match res {
            Ok(resp) => {
                if resp.status().is_success() {
                    Ok(true)
                } else if resp.status().as_u16() == 401 {
                    Err(RouterError::ProviderUnavailable {
                        provider: self.id.clone(),
                        reason: "Authentication failed: 401 Unauthorized (check API key)".to_string(),
                    })
                } else {
                    Ok(false)
                }
            }
            Err(_) => Ok(false),
        }
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        if !self.configured_models.is_empty() {
            return Ok(self.configured_models.clone());
        }

        let url = self.models_endpoint();
        let headers = self.build_headers()?;

        let resp = self
            .client
            .get(&url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| RouterError::Http(format!("List models for '{}' failed: {}", self.id, e)))?;

        if !resp.status().is_success() {
            return Ok(self.configured_models.clone());
        }

        let val: Value = resp.json().await.unwrap_or(Value::Null);
        let mut models = Vec::new();
        if let Some(data) = val.get("data").and_then(|d| d.as_array()) {
            for item in data {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    models.push(id.to_string());
                }
            }
        }

        Ok(models)
    }
}
