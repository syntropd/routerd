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

pub struct MiniMaxAdapter {
    id: String,
    base_url: String,
    api_key: Option<String>,
    client: Client,
    configured_models: Vec<String>,
}

impl MiniMaxAdapter {
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

    fn endpoint(&self) -> String {
        if self.base_url.ends_with("/chat/completions")
            || self.base_url.ends_with("/chatcompletion_v2")
        {
            self.base_url.clone()
        } else if self.base_url.ends_with("/v1") {
            format!("{}/chat/completions", self.base_url)
        } else {
            format!("{}/v1/chat/completions", self.base_url)
        }
    }

    fn build_headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(key) = &self.api_key {
            let auth_val = format!("Bearer {}", key.trim());
            let header_val = HeaderValue::from_str(&auth_val).map_err(|e| {
                RouterError::Credential(format!("Invalid header character in MiniMax API key: {}", e))
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
        if let Some(max_tokens) = req.max_tokens.or(req.max_completion_tokens) {
            body["max_tokens"] = json!(max_tokens);
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
impl ProviderAdapter for MiniMaxAdapter {
    async fn chat_completion(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse> {
        let url = self.endpoint();
        let headers = self.build_headers()?;
        let payload = self.build_payload(target_model, request, false);

        debug!("MiniMax adapter [{}]: POST non-streaming to {}", self.id, url);

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .map_err(|e| RouterError::Http(format!("MiniMax request to '{}' failed: {}", self.id, e)))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(RouterError::ProviderUnavailable {
                provider: self.id.clone(),
                reason: format!("MiniMax HTTP {} - {}", status, err_text),
            });
        }

        let completion: ChatCompletionResponse = resp.json().await.map_err(|e| {
            RouterError::Http(format!("Failed to parse MiniMax JSON reply from '{}': {}", self.id, e))
        })?;

        Ok(completion)
    }

    async fn chat_completion_stream(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ByteStream> {
        let url = self.endpoint();
        let headers = self.build_headers()?;
        let payload = self.build_payload(target_model, request, true);

        debug!("MiniMax adapter [{}]: POST streaming to {}", self.id, url);

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .map_err(|e| RouterError::Http(format!("MiniMax streaming to '{}' failed: {}", self.id, e)))?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(RouterError::ProviderUnavailable {
                provider: self.id.clone(),
                reason: format!("MiniMax streaming HTTP {} - {}", status, err_text),
            });
        }

        let stream = resp
            .bytes_stream()
            .map(|chunk_res| chunk_res.map_err(|e| RouterError::Http(e.to_string())));

        Ok(Box::pin(stream))
    }

    async fn health_check(&self) -> Result<bool> {
        // MiniMax does not require special ping; check base URL reachability
        let url = self.endpoint();
        let headers = self.build_headers()?;
        let res = self
            .client
            .head(&url)
            .headers(headers)
            .timeout(Duration::from_secs(5))
            .send()
            .await;

        match res {
            Ok(resp) => {
                if resp.status().is_success() || resp.status().as_u16() == 405 {
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
        Ok(self.configured_models.clone())
    }
}
