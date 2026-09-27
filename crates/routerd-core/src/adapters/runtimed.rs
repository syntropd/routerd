//! Adapter for the owned runtimed engine over its Varlink socket.
//!
//! Provider kind `"runtimed"`, base URL is the Runtime1 socket path
//! (`/run/syntrop/io.syntrop.Runtime1`). Calls `Generate` for completions;
//! runtimed is single-shot, so the streaming half answers with one SSE
//! chunk followed by `[DONE]`.

use super::{ByteStream, ProviderAdapter};
use crate::config::ProviderConfig;
use crate::error::{Result, RouterError};
use crate::models::{
    ChatChoice, ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, ChatMessage,
    ChunkChoice, ChunkDelta, UsageInfo,
};
use async_trait::async_trait;
use bytes::Bytes;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;
use uuid::Uuid;

pub struct RuntimedAdapter {
    id: String,
    socket_path: PathBuf,
    configured_models: Vec<String>,
    timeout: Duration,
}

impl RuntimedAdapter {
    pub fn new(cfg: &ProviderConfig) -> Self {
        let p = if cfg.base_url.starts_with("varlink:") {
            &cfg.base_url["varlink:".len()..]
        } else {
            &cfg.base_url
        };
        Self {
            id: cfg.id.clone(),
            socket_path: PathBuf::from(p),
            configured_models: cfg.models.iter().map(|m| m.name.clone()).collect(),
            timeout: Duration::from_millis(cfg.timeout_ms.max(1000)),
        }
    }

    fn extract_user_prompt(req: &ChatCompletionRequest) -> String {
        let mut prompt = String::new();
        for msg in &req.messages {
            prompt.push_str(&format!("{}: {}\n", msg.role, msg.content_as_str()));
        }
        prompt
    }

    /// One Varlink call: connect, send, read the single `\0`-framed reply.
    async fn call(&self, method: &str, parameters: Value) -> Result<Value> {
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
            let n = timeout(self.timeout, stream.read(&mut chunk))
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
        let prompt = Self::extract_user_prompt(request);
        debug!("runtimed [{}]: calling Generate for {}", self.id, target_model);
        let max_tokens = request
            .max_tokens
            .or(request.max_completion_tokens)
            .unwrap_or(256);
        let params = json!({
            "model": target_model,
            "prompt": prompt,
            "max_tokens": max_tokens,
            "temperature": request.temperature.unwrap_or(0.0),
            "top_k": 0,
            "top_p": request.top_p.unwrap_or(1.0),
            "seed": 0,
            "image": Value::Null,
        });
        let parameters = self.call("io.syntrop.Runtime1.Generate", params).await?;
        let result = parameters.get("result").cloned().unwrap_or(Value::Null);
        let text = result.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let prompt_tok = result
            .get("prompt_tokens")
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as usize;
        let comp_tok = result
            .get("completion_tokens")
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as usize;
        let finish = result
            .get("finish_reason")
            .and_then(|f| f.as_str())
            .unwrap_or("stop");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Ok(ChatCompletionResponse {
            id: format!("chatcmpl-runtimed-{}", Uuid::new_v4()),
            object: "chat.completion".to_string(),
            created: now,
            model: target_model.to_string(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: Value::String(text.to_string()),
                    name: None,
                },
                finish_reason: Some(finish.to_string()),
            }],
            usage: Some(UsageInfo {
                prompt_tokens: prompt_tok,
                completion_tokens: comp_tok,
                total_tokens: prompt_tok + comp_tok,
            }),
        })
    }

    async fn chat_completion_stream(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ByteStream> {
        let full = self.chat_completion(target_model, request).await?;
        let text = full.choices.first().map(|c| c.message.content_as_str());
        let (tx, rx) = mpsc::channel::<Result<Bytes>>(4);
        let completion_id = full.id.clone();
        let model_str = target_model.to_string();
        let now = full.created;
        tokio::spawn(async move {
            if let Some(text) = text {
                let chunk_obj = ChatCompletionChunk {
                    id: completion_id,
                    object: "chat.completion.chunk".to_string(),
                    created: now,
                    model: model_str,
                    choices: vec![ChunkChoice {
                        index: 0,
                        delta: ChunkDelta {
                            role: None,
                            content: Some(text),
                        },
                        finish_reason: Some("stop".to_string()),
                    }],
                };
                let sse_line = format!(
                    "data: {}\n\n",
                    serde_json::to_string(&chunk_obj).unwrap_or_default()
                );
                let _ = tx.send(Ok(Bytes::from(sse_line))).await;
            }
            let _ = tx.send(Ok(Bytes::from("data: [DONE]\n\n"))).await;
        });
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn health_check(&self) -> Result<bool> {
        if !self.socket_path.exists() {
            return Ok(false);
        }
        match self.call("io.syntrop.Runtime1.GetLoad", json!({})).await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let mut models = Vec::new();
        if self.socket_path.exists() {
            if let Ok(parameters) = self
                .call("io.syntrop.Runtime1.ListLoadedModels", json!({}))
                .await
            {
                if let Some(list) = parameters.get("models").and_then(|m| m.as_array()) {
                    for item in list {
                        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                            models.push(name.to_string());
                        }
                    }
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
