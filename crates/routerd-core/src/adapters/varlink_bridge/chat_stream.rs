use super::VarlinkBridgeAdapter;
use crate::adapters::ByteStream;
use crate::error::{Result, RouterError};
use crate::models::{ChatCompletionChunk, ChatCompletionRequest, ChunkChoice, ChunkDelta};
use bytes::Bytes;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;
use uuid::Uuid;

impl VarlinkBridgeAdapter {
    pub(super) async fn stream_chat(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ByteStream> {
        let prompt = VarlinkBridgeAdapter::extract_user_prompt(request);
        debug!("Varlink bridge [{}]: streaming StreamInference", self.id);

        let stream = timeout(self.timeout, UnixStream::connect(&self.socket_path))
            .await
            .map_err(|_| RouterError::Timeout("Varlink streaming connection timed out".into()))?
            .map_err(|e| RouterError::Varlink(format!("Failed to connect to Varlink socket {:?}: {}", self.socket_path, e)))?;

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

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

        timeout(self.timeout, writer.write_all(&req_bytes))
            .await
            .map_err(|_| RouterError::Timeout("Varlink stream write timed out".into()))?
            .map_err(RouterError::Io)?;

        let (tx, rx) = mpsc::channel::<Result<Bytes>>(32);
        let completion_id = format!("chatcmpl-varlink-{}", Uuid::new_v4());
        let model_str = target_model.to_string();
        let stream_timeout = self.timeout;

        tokio::spawn(async move {
            let mut buf = Vec::with_capacity(512);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            loop {
                buf.clear();
                let read_res = timeout(stream_timeout, reader.read_until(0, &mut buf)).await;
                match read_res {
                    Ok(Ok(0)) => break,
                    Ok(Ok(_)) => {
                        if buf.last() == Some(&0) {
                            buf.pop();
                        }
                        if buf.is_empty() {
                            continue;
                        }

                        let reply_res: std::result::Result<Value, serde_json::Error> =
                            serde_json::from_slice(&buf);

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
                    Ok(Err(e)) => {
                        let _ = tx.send(Err(RouterError::Io(e))).await;
                        break;
                    }
                    Err(_) => {
                        let _ = tx.send(Err(RouterError::Timeout("Varlink streaming chunk timed out".into()))).await;
                        break;
                    }
                }
            }

            // Send standard SSE completion delimiter
            let _ = tx.send(Ok(Bytes::from("data: [DONE]\n\n"))).await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}
