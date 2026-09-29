use crate::adapters::{ByteStream, ProviderAdapter};
use crate::config::ProviderConfig;
use crate::error::{Result, RouterError};
use crate::models::{ChatCompletionRequest, ChatCompletionResponse};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

/// Cold model loads (gigabytes off disk) plus CPU decoding can take
/// minutes. Completions always get at least this long, no matter how
/// snappy the configured per-hop timeout is.
const MIN_GENERATE_TIMEOUT: Duration = Duration::from_secs(600);

pub struct RuntimedAdapter {
    pub(super) id: String,
    pub(super) socket_path: PathBuf,
    pub(super) configured_models: Vec<String>,
    pub(super) timeout: Duration,
    pub(super) generate_timeout: Duration,
}

impl RuntimedAdapter {
    pub fn new(cfg: &ProviderConfig) -> Self {
        let p = if cfg.base_url.starts_with("varlink:") {
            &cfg.base_url["varlink:".len()..]
        } else {
            &cfg.base_url
        };
        let hop = Duration::from_millis(cfg.timeout_ms.max(1000));
        Self {
            id: cfg.id.clone(),
            socket_path: PathBuf::from(p),
            configured_models: cfg.models.iter().map(|m| m.name.clone()).collect(),
            timeout: hop,
            generate_timeout: hop.max(MIN_GENERATE_TIMEOUT),
        }
    }

    pub(super) fn extract_user_prompt(req: &ChatCompletionRequest) -> String {
        let mut prompt = String::new();
        for msg in &req.messages {
            prompt.push_str(&format!("{}: {}\n", msg.role, msg.content_as_str()));
        }
        prompt
    }

    /// One Varlink call: connect, send, read the single `\0`-framed reply.
    pub(super) async fn call(&self, method: &str, parameters: Value) -> Result<Value> {
        self.call_with_timeout(method, parameters, self.timeout).await
    }


    /// Same call with an explicit budget. Completions pass the generous
    /// generate timeout; cheap status probes keep the snappy one.
    pub(super) async fn call_with_timeout(
        &self,
        method: &str,
        parameters: Value,
        budget: Duration,
    ) -> Result<Value> {
        let mut stream = timeout(self.timeout, UnixStream::connect(&self.socket_path))
            .await
            .map_err(|_| RouterError::Timeout("runtimed connection timed out".into()))?
            .map_err(|e| {
                RouterError::Varlink(format!(
                    "Failed to connect to runtimed socket {:?}: {}",
                    self.socket_path, e
                ))
            })?;
        let req = json!({ "method": method, "parameters": parameters });
        let mut req_bytes = serde_json::to_vec(&req)?;
        req_bytes.push(0);
        timeout(self.timeout, stream.write_all(&req_bytes))
            .await
            .map_err(|_| RouterError::Timeout("runtimed write timed out".into()))?
            .map_err(RouterError::Io)?;

        let mut buf = Vec::with_capacity(4096);
        let mut chunk = [0u8; 1024];
        loop {
            let n = timeout(budget, stream.read(&mut chunk))
                .await
                .map_err(|_| RouterError::Timeout("runtimed read timed out".into()))?
                .map_err(RouterError::Io)?;
            if n == 0 {
                return Err(RouterError::Varlink("runtimed closed connection".into()));
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.len() > 1024 * 1024 {
                return Err(RouterError::Varlink("runtimed reply too large".into()));
            }
            if let Some(pos) = buf.iter().position(|&b| b == 0) {
                let reply: Value = serde_json::from_slice(&buf[..pos])?;
                if let Some(err) = reply.get("error").and_then(|e| e.as_str()) {
                    return Err(RouterError::Varlink(format!("runtimed error: {}", err)));
                }
                return reply.get("parameters").cloned().ok_or_else(|| {
                    RouterError::Varlink("runtimed reply missing parameters".into())
                });
            }
        }
    }
}

#[async_trait]
impl ProviderAdapter for RuntimedAdapter {
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
