use super::openai::OpenAICompatibleAdapter;
use crate::config::ProviderConfig;
use crate::models::{ChatCompletionRequest, ChatMessage};

fn adapter(base_url: &str) -> OpenAICompatibleAdapter {
    OpenAICompatibleAdapter::new(&ProviderConfig {
        id: "o".to_string(),
        name: "OpenAI".to_string(),
        kind: "openai".to_string(),
        base_url: base_url.to_string(),
        api_key: None,
        tier: "fast".to_string(),
        weight: 1.0,
        enabled: true,
        timeout_ms: 1000,
        models: vec![],
    })
}

fn request() -> ChatCompletionRequest {
    ChatCompletionRequest {
        model: "m".to_string(),
        messages: vec![ChatMessage::new("user", serde_json::json!("hi"))],
        max_tokens: Some(7),
        max_completion_tokens: Some(9),
        stream: Some(false),
        ..Default::default()
    }
}

#[test]
fn chat_endpoint_appends_only_when_missing() {
    assert_eq!(
        adapter("https://x/v1").chat_endpoint(),
        "https://x/v1/chat/completions"
    );
    assert_eq!(
        adapter("https://x/openai").chat_endpoint(),
        "https://x/openai/chat/completions"
    );
    assert_eq!(
        adapter("https://x/v1/chat/completions").chat_endpoint(),
        "https://x/v1/chat/completions"
    );
    assert_eq!(
        adapter("https://x/custom").chat_endpoint(),
        "https://x/custom/v1/chat/completions"
    );
}

#[test]
fn models_endpoint_appends_only_when_missing() {
    assert_eq!(adapter("https://x/v1").models_endpoint(), "https://x/v1/models");
    assert_eq!(
        adapter("https://x/models").models_endpoint(),
        "https://x/models"
    );
}

#[test]
fn payload_carries_both_token_caps() {
    let body = adapter("https://x/v1").build_payload("m", &request(), false);
    assert_eq!(body["max_tokens"], 7);
    assert_eq!(body["max_completion_tokens"], 9);
    assert_eq!(body["stream"], false);
}

#[test]
fn payload_carries_tools_and_tool_choice() {
    let mut req = request();
    req.tools = Some(vec![crate::models::ToolDefinition::function(
        "unit_status",
        Some("check status".into()),
        Some(serde_json::json!({"type": "object"})),
    )]);
    req.tool_choice = Some(serde_json::json!("auto"));
    let body = adapter("https://x/v1").build_payload("m", &req, false);
    assert_eq!(body["tools"][0]["function"]["name"], "unit_status");
    assert_eq!(body["tool_choice"], "auto");
}

#[test]
fn response_deserializes_tool_calls_into_chat_message() {
    let raw = serde_json::json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 12345,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_123",
                    "type": "function",
                    "function": {
                        "name": "unit_status",
                        "arguments": "{\"unit\":\"dbus\"}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });
    let parsed: crate::models::ChatCompletionResponse = serde_json::from_value(raw).unwrap();
    assert_eq!(parsed.choices[0].message.role, "assistant");
    let calls = parsed.choices[0].message.tool_calls.as_ref().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id.as_deref(), Some("call_123"));
    assert_eq!(calls[0].function.name.as_deref(), Some("unit_status"));
}

