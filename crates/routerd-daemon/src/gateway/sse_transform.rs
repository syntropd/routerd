//! SSE stream and response transformer separating reasoning tokens into `delta.reasoning_content`.

use bytes::Bytes;
use futures::StreamExt;
use routerd_core::adapters::ByteStream;
use routerd_core::models::ChatCompletionResponse;
use routerd_core::wire::{FilteredItem, ThinkFilter};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

/// Clean non-streaming ChatCompletionResponse by separating <think>...</think> into reasoning_content.
pub fn clean_completion_response(response: &mut ChatCompletionResponse) {
    for choice in &mut response.choices {
        let text = choice.message.content_as_str();
        if text.contains("<think>") || text.contains("</think>") {
            let mut filter = ThinkFilter::new();
            let mut items = filter.process(&text);
            items.extend(filter.flush());
            let mut clean_content = String::new();
            let mut reasoning = String::new();
            for item in items {
                match item {
                    FilteredItem::Content(c) => clean_content.push_str(&c),
                    FilteredItem::Reasoning(r) => reasoning.push_str(&r),
                }
            }
            choice.message.content = Value::String(if !clean_content.is_empty() {
                clean_content
            } else if choice.finish_reason.as_deref() == Some("length") {
                if !reasoning.trim().is_empty() { reasoning.clone() } else { "[Response truncated during reasoning due to token limit]".to_string() }
            } else {
                clean_content
            });
            if !reasoning.is_empty() {
                choice.message.reasoning_content = match &choice.message.reasoning_content {
                    Some(prev) => Some(format!("{prev}{reasoning}")),
                    None => Some(reasoning),
                };
            }
        }
    }
}

/// Transform streaming SSE bytes, segregating <think> tags into delta.reasoning_content.
pub fn transform_sse_stream(mut input: ByteStream) -> ByteStream {
    let (tx, rx) = mpsc::channel(32);
    tokio::spawn(async move {
        let mut filter = ThinkFilter::new();
        let mut buffer = String::new();
        while let Some(chunk_res) = input.next().await {
            let bytes = match chunk_res {
                Ok(b) => b,
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            while let Some((pos, len)) = find_sse_event_boundary(&buffer) {
                let event = buffer[..pos].to_string();
                buffer.drain(..pos + len);
                if !process_sse_event(&event, &mut filter, &tx).await {
                    return;
                }
            }
        }
        if !buffer.trim().is_empty() {
            let leftover = std::mem::take(&mut buffer);
            let _ = process_sse_event(&leftover, &mut filter, &tx).await;
        }
        for item in filter.flush() {
            if let Some(line) = synthetic_chunk_line(item) {
                if tx.send(Ok(Bytes::from(line))).await.is_err() {
                    return;
                }
            }
        }
    });
    Box::pin(ReceiverStream::new(rx))
}

async fn process_sse_event(
    event: &str,
    filter: &mut ThinkFilter,
    tx: &mpsc::Sender<Result<Bytes, routerd_core::RouterError>>,
) -> bool {
    for line in event.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("data:") {
            if !trimmed.is_empty() {
                let out = format!("{trimmed}\n\n");
                if tx.send(Ok(Bytes::from(out))).await.is_err() {
                    return false;
                }
            }
            continue;
        }
        let payload = trimmed["data:".len()..].trim();
        if payload == "[DONE]" {
            for item in filter.flush() {
                if let Some(chunk_line) = synthetic_chunk_line(item) {
                    if tx.send(Ok(Bytes::from(chunk_line))).await.is_err() {
                        return false;
                    }
                }
            }
            return tx.send(Ok(Bytes::from("data: [DONE]\n\n"))).await.is_ok();
        }
        let Ok(mut val) = serde_json::from_str::<Value>(payload) else {
            let out = format!("data: {payload}\n\n");
            return tx.send(Ok(Bytes::from(out))).await.is_ok();
        };
        if let Some(choices) = val.get_mut("choices").and_then(|c| c.as_array_mut()) {
            if let Some(first) = choices.first_mut() {
                let has_finish = first.get("finish_reason").is_some();
                if let Some(delta) = first.get_mut("delta") {
                    let content_opt = delta.get("content").and_then(|c| c.as_str()).map(str::to_string);
                    if let Some(raw_content) = content_opt {
                        let items = filter.process(&raw_content);
                        if items.is_empty() {
                            let has_role = delta.get("role").is_some();
                            if let Some(obj) = delta.as_object_mut() {
                                obj.remove("content");
                            }
                            if has_finish || has_role {
                                let out = format!("data: {}\n\n", serde_json::to_string(&val).unwrap_or_default());
                                if tx.send(Ok(Bytes::from(out))).await.is_err() {
                                    return false;
                                }
                            }
                            continue;
                        }
                        for item in items {
                            let mut clone_val = val.clone();
                            if let Some(d) = clone_val["choices"][0]["delta"].as_object_mut() {
                                match item {
                                    FilteredItem::Reasoning(r) => {
                                        d.remove("content");
                                        d.insert("reasoning_content".to_string(), json!(r));
                                    }
                                    FilteredItem::Content(c) => {
                                        d.insert("content".to_string(), json!(c));
                                    }
                                }
                            }
                            let out = format!("data: {}\n\n", serde_json::to_string(&clone_val).unwrap_or_default());
                            if tx.send(Ok(Bytes::from(out))).await.is_err() {
                                return false;
                            }
                        }
                        continue;
                    }
                }
            }
        }
        let out = format!("data: {}\n\n", serde_json::to_string(&val).unwrap_or_default());
        if tx.send(Ok(Bytes::from(out))).await.is_err() {
            return false;
        }
    }
    true
}

fn synthetic_chunk_line(item: FilteredItem) -> Option<String> {
    let (c, r) = match item {
        FilteredItem::Content(c) => (Some(c), None),
        FilteredItem::Reasoning(r) => (None, Some(r)),
    };
    let val = json!({
        "choices": [{
            "index": 0,
            "delta": {
                "content": c,
                "reasoning_content": r
            }
        }]
    });
    Some(format!("data: {}\n\n", serde_json::to_string(&val).ok()?))
}

fn find_sse_event_boundary(buf: &str) -> Option<(usize, usize)> {
    let crlf = buf.find("\r\n\r\n").map(|p| (p, 4));
    let lf = buf.find("\n\n").map(|p| (p, 2));
    match (crlf, lf) {
        (Some((p1, l1)), Some((p2, l2))) => Some(if p1 <= p2 { (p1, l1) } else { (p2, l2) }),
        (Some(c), None) => Some(c),
        (None, Some(l)) => Some(l),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use routerd_core::models::{ChatChoice, ChatMessage};

    #[test]
    fn test_find_sse_event_boundary() {
        assert_eq!(find_sse_event_boundary("data: 1\n\ndata: 2"), Some((7, 2)));
        assert_eq!(find_sse_event_boundary("data: 1\r\n\r\ndata: 2"), Some((7, 4)));
        assert_eq!(find_sse_event_boundary("incomplete"), None);
    }

    #[test]
    fn test_clean_completion_response_with_think() {
        let mut resp = ChatCompletionResponse {
            id: "id1".into(),
            object: "chat.completion".into(),
            created: 0,
            model: "m".into(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage::new("assistant", "<think>thought step</think>final answer"),
                finish_reason: Some("stop".into()),
            }],
            usage: None,
        };
        clean_completion_response(&mut resp);
        assert_eq!(resp.choices[0].message.content_as_str(), "final answer");
        assert_eq!(resp.choices[0].message.reasoning_content.as_deref(), Some("thought step"));
    }

    #[test]
    fn test_clean_completion_swallow_leading_end_think() {
        let mut resp = ChatCompletionResponse {
            id: "id2".into(),
            object: "chat.completion".into(),
            created: 0,
            model: "m".into(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage::new("assistant", "</think>clean response"),
                finish_reason: Some("stop".into()),
            }],
            usage: None,
        };
        clean_completion_response(&mut resp);
        assert_eq!(resp.choices[0].message.content_as_str(), "clean response");
        assert_eq!(resp.choices[0].message.reasoning_content, None);
    }

    #[test]
    fn test_clean_completion_truncation_during_reasoning() {
        let mut resp = ChatCompletionResponse {
            id: "id3".into(), object: "chat.completion".into(), created: 0, model: "m".into(),
            choices: vec![ChatChoice {
                index: 0, message: ChatMessage::new("assistant", "<think>truncated thought"),
                finish_reason: Some("length".into()),
            }], usage: None,
        };
        clean_completion_response(&mut resp);
        assert_eq!(resp.choices[0].message.content_as_str(), "truncated thought");
    }
}
