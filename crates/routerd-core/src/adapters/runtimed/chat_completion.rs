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
        let mut prompt = Self::extract_user_prompt(request);
        if let Some(tools) = &request.tools {
            if !tools.is_empty() {
                if let Ok(tools_json) = serde_json::to_string(tools) {
                    prompt.push_str(&format!("\nTools: {}\n", tools_json));
                }
            }
        }
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
        if request.tools.as_ref().map_or(false, |t| !t.is_empty()) {
            params["grammar_type"] = json!("json");
        }
        if let Some(budget) = request.reasoning_budget() {
            params["reasoning_budget"] = json!(budget);
        }
        if let Some(effort) = request.reasoning_effort {
            params["reasoning_effort"] = json!(effort.as_str());
        }
        let gen_timeout = self.compute_generate_timeout(request);
        let parameters = self
            .call_with_timeout("io.syntrop.Runtime1.Generate", params, gen_timeout)
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
        let mut reasoning_content = String::new();
        for item in items {
            match item {
                crate::wire::FilteredItem::Content(c) => clean_content.push_str(&c),
                crate::wire::FilteredItem::Reasoning(r) => reasoning_content.push_str(&r),
            }
        }
        let final_content = if !clean_content.is_empty() {
            clean_content
        } else if finish == "length" {
            if !reasoning_content.trim().is_empty() {
                reasoning_content.clone()
            } else {
                "[Response truncated during reasoning due to token limit]".to_string()
            }
        } else if !text.is_empty() && reasoning_content.is_empty() {
            text.to_string()
        } else {
            clean_content
        };
        let final_reasoning = if reasoning_content.is_empty() {
            None
        } else {
            Some(reasoning_content)
        };
        let mut tool_calls = None;
        let mut finish_reason = finish.to_string();
        if request.tools.as_ref().map_or(false, |t| !t.is_empty()) {
            if let Ok(val) = serde_json::from_str::<Value>(&final_content) {
                if let Some(calls) = val.get("tool_calls").and_then(|v| serde_json::from_value::<Vec<crate::models::ToolCall>>(v.clone()).ok()) {
                    tool_calls = Some(calls);
                    finish_reason = "tool_calls".to_string();
                } else if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
                    let args = val.get("arguments").map(|a| if a.is_string() { a.as_str().unwrap().to_string() } else { a.to_string() }).unwrap_or_else(|| "{}".to_string());
                    let call_id = format!("call_{}", &Uuid::new_v4().to_string()[..8]);
                    tool_calls = Some(vec![crate::models::ToolCall::function(call_id, name, args)]);
                    finish_reason = "tool_calls".to_string();
                }
            }
        }
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
                    reasoning_content: final_reasoning,
                    tool_calls: tool_calls.clone(),
                    tool_call_id: None,
                },
                tool_calls,
                finish_reason: Some(finish_reason),
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
        let first_choice = full.choices.first().cloned();
        let (tx, rx) = mpsc::channel::<Result<Bytes>>(4);
        let completion_id = full.id.clone();
        let model_str = target_model.to_string();
        let now = full.created;
        tokio::spawn(async move {
            if let Some(choice) = first_choice {
                let mut sent_role = false;
                if let Some(reasoning) = choice.message.reasoning_content.as_deref() {
                    if !reasoning.is_empty() {
                        let chunk = ChatCompletionChunk {
                            id: completion_id.clone(),
                            object: "chat.completion.chunk".to_string(),
                            created: now,
                            model: model_str.clone(),
                            choices: vec![ChunkChoice {
                                index: 0,
                                delta: ChunkDelta {
                                    role: Some("assistant".to_string()),
                                    content: None,
                                    reasoning_content: Some(reasoning.to_string()),
                                    tool_calls: None,
                                },
                                tool_calls: None,
                                finish_reason: None,
                            }],
                        };
                        sent_role = true;
                        let line = format!("data: {}\n\n", serde_json::to_string(&chunk).unwrap_or_default());
                        let _ = tx.send(Ok(Bytes::from(line))).await;
                    }
                }
                let text = choice.message.content_as_str();
                if !text.is_empty() || choice.tool_calls.is_some() {
                    let chunk = ChatCompletionChunk {
                        id: completion_id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created: now,
                        model: model_str.clone(),
                        choices: vec![ChunkChoice {
                            index: 0,
                            delta: ChunkDelta {
                                role: (!sent_role).then(|| "assistant".to_string()),
                                content: if text.is_empty() { None } else { Some(text) },
                                reasoning_content: None,
                                tool_calls: choice.tool_calls.clone(),
                            },
                            tool_calls: choice.tool_calls.clone(),
                            finish_reason: None,
                        }],
                    };
                    let line = format!("data: {}\n\n", serde_json::to_string(&chunk).unwrap_or_default());
                    let _ = tx.send(Ok(Bytes::from(line))).await;
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
                            tool_calls: None,
                        },
                        tool_calls: None,
                        finish_reason: choice.finish_reason.or_else(|| Some("stop".to_string())),
                    }],
                };
                let line = format!("data: {}\n\n", serde_json::to_string(&final_chunk).unwrap_or_default());
                let _ = tx.send(Ok(Bytes::from(line))).await;
            }
            let _ = tx.send(Ok(Bytes::from("data: [DONE]\n\n"))).await;
        });
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}
