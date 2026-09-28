use super::VarlinkBridgeAdapter;
use crate::error::{Result, RouterError};
use crate::models::{
    ChatChoice, ChatCompletionRequest, ChatCompletionResponse, ChatMessage, UsageInfo,
};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::timeout;
use tracing::debug;
use uuid::Uuid;

impl VarlinkBridgeAdapter {
    pub(super) async fn complete_chat(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse> {
        let prompt = VarlinkBridgeAdapter::extract_user_prompt(request);
        debug!("Varlink bridge [{}]: calling StreamInference non-streaming", self.id);

        let stream = timeout(self.timeout, UnixStream::connect(&self.socket_path))
            .await
            .map_err(|_| RouterError::Timeout("Varlink connection timed out".into()))?
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
            .map_err(|_| RouterError::Timeout("Varlink write timed out".into()))?
            .map_err(RouterError::Io)?;

        let mut accumulated_text = String::new();
        let mut buf = Vec::with_capacity(512);

        loop {
            buf.clear();
            let n = timeout(self.timeout, reader.read_until(0, &mut buf))
                .await
                .map_err(|_| RouterError::Timeout("Varlink bridge read timed out".into()))?
                .map_err(RouterError::Io)?;
            if n == 0 {
                break;
            }
            if buf.last() == Some(&0) {
                buf.pop();
            }
            if buf.is_empty() {
                continue;
            }

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
            if !continues {
                break;
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
}
