use super::RuntimedAdapter;
use crate::adapters::ByteStream;
use crate::error::Result;
use crate::models::{
    ChatChoice, ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, ChatMessage,
    ChunkChoice, ChunkDelta, UsageInfo,
};
use bytes::Bytes;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;
use uuid::Uuid;

impl RuntimedAdapter {
    pub(super) async fn complete_chat(
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
        let mut params = json!({
            "model": target_model,
            "prompt": prompt,
            "max_tokens": max_tokens,
            "temperature": request.temperature.unwrap_or(0.0),
            "top_k": 0,
            "top_p": request.top_p.unwrap_or(1.0),
            "seed": 0,
            "image": Value::Null,
        });
        if let Some(budget) = request.reasoning_budget() {
            params["reasoning_budget"] = json!(budget);
        }
        let parameters = self
            .call_with_timeout("io.syntrop.Runtime1.Generate", params, self.generate_timeout)
            .await?;
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
        let mut filter = crate::wire::ThinkFilter::new();
        let mut items = filter.process(text);
        items.extend(filter.flush());
        let mut clean_content = String::new();
        for item in items {
            if let crate::wire::FilteredItem::Content(c) = item {
                clean_content.push_str(&c);
            }
        }
        let final_content = if clean_content.is_empty() && !text.is_empty() {
            text.to_string()
        } else {
            clean_content
        };
        Ok(ChatCompletionResponse {
            id: format!("chatcmpl-runtimed-{}", Uuid::new_v4()),
            object: "chat.completion".to_string(),
            created: now,
            model: target_model.to_string(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: Value::String(final_content),
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

    pub(super) async fn stream_chat(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ByteStream> {
        let full = self.complete_chat(target_model, request).await?;
        let text = full.choices.first().map(|c| c.message.content_as_str());
        let (tx, rx) = mpsc::channel::<Result<Bytes>>(4);
        let completion_id = full.id.clone();
        let model_str = target_model.to_string();
        let now = full.created;
        tokio::spawn(async move {
            if let Some(text) = text {
                let mut filter = crate::wire::ThinkFilter::new();
                let mut items = filter.process(&text);
                items.extend(filter.flush());
                for (i, item) in items.into_iter().enumerate() {
                    let (content, reasoning) = match item {
                        crate::wire::FilteredItem::Content(c) => (Some(c), None),
                        crate::wire::FilteredItem::Reasoning(r) => (None, Some(r)),
                    };
                    let chunk_obj = ChatCompletionChunk {
                        id: completion_id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created: now,
                        model: model_str.clone(),
                        choices: vec![ChunkChoice {
                            index: 0,
                            delta: ChunkDelta {
                                role: (i == 0).then(|| "assistant".to_string()),
                                content,
                                reasoning_content: reasoning,
                            },
                            finish_reason: None,
                        }],
                    };
                    let sse_line = format!(
                        "data: {}\n\n",
                        serde_json::to_string(&chunk_obj).unwrap_or_default()
                    );
                    let _ = tx.send(Ok(Bytes::from(sse_line))).await;
                }
                let final_chunk = ChatCompletionChunk {
                    id: completion_id,
                    object: "chat.completion.chunk".to_string(),
                    created: now,
                    model: model_str,
                    choices: vec![ChunkChoice {
                        index: 0,
                        delta: ChunkDelta {
                            role: None,
                            content: None,
                            reasoning_content: None,
                        },
                        finish_reason: Some("stop".to_string()),
                    }],
                };
                let sse_line = format!(
                    "data: {}\n\n",
                    serde_json::to_string(&final_chunk).unwrap_or_default()
                );
                let _ = tx.send(Ok(Bytes::from(sse_line))).await;
            }
            let _ = tx.send(Ok(Bytes::from("data: [DONE]\n\n"))).await;
        });
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}
