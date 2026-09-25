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

pub struct VarlinkBridgeAdapter {
    id: String,
    socket_path: PathBuf,
    configured_models: Vec<String>,
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

        Self {
            id: cfg.id.clone(),
            socket_path,
            configured_models,
        }
    }

    fn extract_user_prompt(req: &ChatCompletionRequest) -> String {
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
        let prompt = Self::extract_user_prompt(request);
        debug!("Varlink bridge [{}]: calling StreamInference non-streaming", self.id);

        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| RouterError::Varlink(format!("Failed to connect to Varlink socket {:?}: {}", self.socket_path, e)))?;

        let (mut reader, mut writer) = stream.into_split();

        let req = json!({
            "method": "io.syntrop.Inference1.StreamInference",
            "parameters": {
                "prompt": prompt,
                "model": target_model
            },
            "more": true
        });

        let mut req_bytes = serde_json::to_vec(&req)?;
        req_bytes.push(0);

        writer.write_all(&req_bytes).await.map_err(RouterError::Io)?;

        let mut accumulated_text = String::new();
        let mut buf = Vec::with_capacity(512);
        let mut byte = [0u8; 1];

        loop {
            let n = reader.read(&mut byte).await.map_err(RouterError::Io)?;
            if n == 0 {
                break;
            }
            if byte[0] == 0 {
                if !buf.is_empty() {
                    let reply: Value = serde_json::from_slice(&buf)?;
                    if let Some(err) = reply.get("error").and_then(|e| e.as_str()) {
                        return Err(RouterError::Varlink(format!("Varlink error: {}", err)));
                    }
                    if let Some(chunk) = reply
                        .get("parameters")
                        .and_then(|p| p.get("chunk"))
                        .and_then(|c| c.as_str())
                    {
                        accumulated_text.push_str(chunk);
                    }
                    let continues = reply
                        .get("continues")
                        .and_then(|c| c.as_bool())
                        .unwrap_or(false);
                    buf.clear();
                    if !continues {
                        break;
                    }
                }
            } else {
                buf.push(byte[0]);
            }
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let prompt_tok = request.estimate_prompt_tokens();
        let comp_tok = (accumulated_text.len() as f64 / 3.8).ceil() as usize;

        Ok(ChatCompletionResponse {
            id: format!("chatcmpl-varlink-{}", Uuid::new_v4()),
            object: "chat.completion".to_string(),
            created: now,
            model: target_model.to_string(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: Value::String(accumulated_text),
                    name: None,
                },
                finish_reason: Some("stop".to_string()),
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
        let prompt = Self::extract_user_prompt(request);
        debug!("Varlink bridge [{}]: streaming StreamInference", self.id);

        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| RouterError::Varlink(format!("Failed to connect to Varlink socket {:?}: {}", self.socket_path, e)))?;

        let (mut reader, mut writer) = stream.into_split();

        let req = json!({
            "method": "io.syntrop.Inference1.StreamInference",
            "parameters": {
                "prompt": prompt,
                "model": target_model
            },
            "more": true
        });

        let mut req_bytes = serde_json::to_vec(&req)?;
        req_bytes.push(0);

        writer.write_all(&req_bytes).await.map_err(RouterError::Io)?;

        let (tx, rx) = mpsc::channel::<Result<Bytes>>(32);
        let completion_id = format!("chatcmpl-varlink-{}", Uuid::new_v4());
        let model_str = target_model.to_string();

        tokio::spawn(async move {
            let mut buf = Vec::with_capacity(512);
            let mut byte = [0u8; 1];
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            loop {
                match reader.read(&mut byte).await {
                    Ok(0) => break,
                    Ok(_) => {
                        if byte[0] == 0 {
                            if !buf.is_empty() {
                                let reply_res: std::result::Result<Value, serde_json::Error> =
                                    serde_json::from_slice(&buf);
                                buf.clear();

                                match reply_res {
                                    Ok(reply) => {
                                        if let Some(err) = reply.get("error").and_then(|e| e.as_str()) {
                                            let _ = tx
                                                .send(Err(RouterError::Varlink(format!("Varlink stream error: {}", err))))
                                                .await;
                                            break;
                                        }

                                        let continues = reply
                                            .get("continues")
                                            .and_then(|c| c.as_bool())
                                            .unwrap_or(false);

                                        if let Some(chunk_text) = reply
                                            .get("parameters")
                                            .and_then(|p| p.get("chunk"))
                                            .and_then(|c| c.as_str())
                                        {
                                            let chunk_obj = ChatCompletionChunk {
                                                id: completion_id.clone(),
                                                object: "chat.completion.chunk".to_string(),
                                                created: now,
                                                model: model_str.clone(),
                                                choices: vec![ChunkChoice {
                                                    index: 0,
                                                    delta: ChunkDelta {
                                                        role: None,
                                                        content: Some(chunk_text.to_string()),
                                                    },
                                                    finish_reason: if continues { None } else { Some("stop".to_string()) },
                                                }],
                                            };

                                            let sse_line = format!(
                                                "data: {}\n\n",
                                                serde_json::to_string(&chunk_obj).unwrap_or_default()
                                            );
                                            if tx.send(Ok(Bytes::from(sse_line))).await.is_err() {
                                                break;
                                            }
                                        }

                                        if !continues {
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx.send(Err(RouterError::Json(e))).await;
                                        break;
                                    }
                                }
                            }
                        } else {
                            buf.push(byte[0]);
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(RouterError::Io(e))).await;
                        break;
                    }
                }
            }

            // Send standard SSE completion delimiter
            let _ = tx.send(Ok(Bytes::from("data: [DONE]\n\n"))).await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn health_check(&self) -> Result<bool> {
        if !self.socket_path.exists() {
            return Ok(false);
        }

        let res = timeout(
            Duration::from_millis(1000),
            UnixStream::connect(&self.socket_path),
        )
        .await;

        match res {
            Ok(Ok(mut stream)) => {
                let req = json!({
                    "method": "org.varlink.service.GetInfo",
                    "parameters": {}
                });
                let mut b = serde_json::to_vec(&req)?;
                b.push(0);
                let _ = stream.write_all(&b).await;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        if !self.socket_path.exists() {
            return Ok(self.configured_models.clone());
        }

        let stream_res = timeout(
            Duration::from_millis(1500),
            UnixStream::connect(&self.socket_path),
        )
        .await;

        let mut stream = match stream_res {
            Ok(Ok(s)) => s,
            _ => return Ok(self.configured_models.clone()),
        };

        let req = json!({
            "method": "io.syntrop.Inference1.ListModels",
            "parameters": {}
        });
        let mut b = serde_json::to_vec(&req)?;
        b.push(0);

        if stream.write_all(&b).await.is_err() {
            return Ok(self.configured_models.clone());
        }

        let mut buf = Vec::with_capacity(512);
        let mut byte = [0u8; 1];

        let read_res = timeout(Duration::from_millis(1500), async {
            loop {
                let n = stream.read(&mut byte).await?;
                if n == 0 || byte[0] == 0 {
                    break;
                }
                buf.push(byte[0]);
            }
            Ok::<(), std::io::Error>(())
        })
        .await;

        if read_res.is_err() || buf.is_empty() {
            return Ok(self.configured_models.clone());
        }

        let val: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
        let mut models = Vec::new();
        if let Some(list) = val
            .get("parameters")
            .and_then(|p| p.get("models"))
            .and_then(|m| m.as_array())
        {
            for item in list {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    models.push(id.to_string());
                } else if let Some(s) = item.as_str() {
                    models.push(s.to_string());
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
