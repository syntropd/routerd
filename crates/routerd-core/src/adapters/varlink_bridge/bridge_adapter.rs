use crate::adapters::{ByteStream, ProviderAdapter};
use crate::config::ProviderConfig;
use crate::error::Result;
use crate::models::{ChatCompletionRequest, ChatCompletionResponse};
use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Duration;

pub struct VarlinkBridgeAdapter {
    pub(super) id: String,
    pub(super) socket_path: PathBuf,
    pub(super) configured_models: Vec<String>,
    pub(super) timeout: Duration,
}

impl VarlinkBridgeAdapter {
    pub fn new(cfg: &ProviderConfig) -> Self {
        let p = if cfg.base_url.starts_with("varlink:") {
            &cfg.base_url["varlink:".len()..]
        } else {
            &cfg.base_url
        };
        let socket_path = PathBuf::from(p);
        let configured_models = cfg.models.iter().map(|m| m.name.clone()).collect();
        let timeout = Duration::from_millis(cfg.timeout_ms.max(1000));

        Self {
            id: cfg.id.clone(),
            socket_path,
            configured_models,
            timeout,
        }
    }

    pub(super) fn extract_user_prompt(req: &ChatCompletionRequest) -> String {
        let mut prompt = String::new();
        for msg in &req.messages {
            let role = &msg.role;
            let content = msg.content_as_str();
            prompt.push_str(&format!("{}: {}\n", role, content));
        }
        prompt
    }
}

#[async_trait]
impl ProviderAdapter for VarlinkBridgeAdapter {
    async fn chat_completion(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse> {
        self.complete_chat(target_model, request).await
    }

    async fn chat_completion_stream(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ByteStream> {
        self.stream_chat(target_model, request).await
    }

    async fn health_check(&self) -> Result<bool> {
        self.check_health().await
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        self.available_models().await
    }
}
