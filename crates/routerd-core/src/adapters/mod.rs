use crate::config::ProviderConfig;
use crate::error::Result;
use crate::models::{ChatCompletionRequest, ChatCompletionResponse};
use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;
use std::pin::Pin;
use std::sync::Arc;

pub mod minimax;
pub mod ollama;
pub mod openai;
pub mod varlink_bridge;

pub use minimax::MiniMaxAdapter;
pub use ollama::OllamaAdapter;
pub use openai::OpenAICompatibleAdapter;
pub use varlink_bridge::VarlinkBridgeAdapter;

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>;

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    /// Send chat completion request, getting full non-streaming response.
    async fn chat_completion(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse>;

    /// Send streaming chat completion request, receiving SSE byte stream.
    async fn chat_completion_stream(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ByteStream>;

    /// Health check provider endpoint.
    async fn health_check(&self) -> Result<bool>;

    /// List models available on this provider.
    async fn list_models(&self) -> Result<Vec<String>>;
}

/// Factory function creating appropriate adapter for provider config.
pub fn create_adapter(config: &ProviderConfig) -> Arc<dyn ProviderAdapter> {
    match config.kind.to_ascii_lowercase().as_str() {
        "minimax" => Arc::new(MiniMaxAdapter::new(config)),
        "ollama" => Arc::new(OllamaAdapter::new(config)),
        "varlink" | "syntrop" => Arc::new(VarlinkBridgeAdapter::new(config)),
        _ => Arc::new(OpenAICompatibleAdapter::new(config)),
    }
}
