use super::openai::OpenAICompatibleAdapter;
use crate::config::ProviderConfig;
use crate::models::{ChatCompletionRequest, ChatMessage};
use std::collections::HashMap;

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
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: serde_json::json!("hi"),
            name: None,
        }],
        temperature: None,
        top_p: None,
        max_tokens: Some(7),
        max_completion_tokens: Some(9),
        stream: None,
        tier: None,
        extra: HashMap::new(),
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
