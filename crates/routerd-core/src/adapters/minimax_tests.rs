use super::minimax::MiniMaxAdapter;
use crate::config::ProviderConfig;
use crate::models::{ChatCompletionRequest, ChatMessage};

fn adapter(base_url: &str, api_key: Option<&str>) -> MiniMaxAdapter {
    MiniMaxAdapter::new(&ProviderConfig {
        id: "mm".to_string(),
        name: "MiniMax".to_string(),
        kind: "minimax".to_string(),
        base_url: base_url.to_string(),
        api_key: api_key.map(str::to_string),
        tier: "hard".to_string(),
        weight: 1.0,
        enabled: true,
        timeout_ms: 1000,
        models: vec![],
    })
}

#[test]
fn endpoint_appends_only_when_missing() {
    assert_eq!(adapter("https://x/v1", None).endpoint(), "https://x/v1/chat/completions");
    assert_eq!(
        adapter("https://x/v1/chat/completions", None).endpoint(),
        "https://x/v1/chat/completions"
    );
    assert_eq!(
        adapter("https://x/chatcompletion_v2", None).endpoint(),
        "https://x/chatcompletion_v2"
    );
}

#[test]
fn headers_carry_bearer_only_with_key() {
    let h = adapter("https://x/v1", Some("k")).build_headers().unwrap();
    assert_eq!(h.get("authorization").unwrap(), "Bearer k");
    let h = adapter("https://x/v1", None).build_headers().unwrap();
    assert!(h.get("authorization").is_none());
    assert_eq!(h.get("content-type").unwrap(), "application/json");
}

#[test]
fn payload_passes_extras_except_reserved() {
    let req = ChatCompletionRequest {
        model: "m".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: serde_json::json!("hi"),
            name: None,
        }],
        temperature: Some(0.5),
        top_p: None,
        max_tokens: Some(10),
        max_completion_tokens: None,
        stream: None,
        tier: None,
        extra: [("tier".to_string(), serde_json::json!("fast"))].into_iter().collect(),
    };
    let body = adapter("https://x/v1", None).build_payload("m", &req, true);
    assert_eq!(body["stream"], true);
    assert_eq!(body["temperature"], 0.5);
    assert!(body.get("tier").is_none());
}
