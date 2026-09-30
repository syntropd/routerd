use super::RuntimedAdapter;
use crate::adapters::ByteStream;
use crate::error::Result;
use crate::models::{
    ChatCompletionChunk, ChatCompletionRequest, ChunkChoice, ChunkDelta,
};
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

impl RuntimedAdapter {
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
