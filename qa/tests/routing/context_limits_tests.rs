use routerd_core::{ChatCompletionRequest, ChatMessage, RequestProfile};
use serde_json::json;

#[test]
fn test_token_estimation_and_limits() {
    let req = ChatCompletionRequest {
        model: "router:fast".to_string(),
        messages: vec![
            ChatMessage::new("system", json!("You are a helpful Linux assistant.")),
            ChatMessage::new("user", json!("Explain how systemd socket activation works in pure Rust.")),
        ],
        max_tokens: Some(1024),
        ..Default::default()
    };

    let prompt_est = req.estimate_prompt_tokens();
    assert!(prompt_est > 10 && prompt_est < 60);

    let total_est = req.estimate_total_tokens();
    assert_eq!(total_est, prompt_est + 1024);
}

#[test]
fn test_zero_cross_chat_context_contamination() {
    // Verify request lifecycle is completely isolated and no state persists
    let req1 = ChatCompletionRequest {
        model: "router:fast".to_string(),
        messages: vec![ChatMessage::new("user", json!("Secret confidential prompt 1"))],
        max_tokens: Some(128),
        ..Default::default()
    };

    let p1 = RequestProfile::from_request(&req1, "fast");
    let prompt1_len = req1.estimate_prompt_tokens();
    drop(req1); // Immediate drop of request 1

    let req2 = ChatCompletionRequest {
        model: "router:fast".to_string(),
        messages: vec![ChatMessage::new(
            "user",
            json!("Public prompt 2 with different length content here"),
        )],
        max_tokens: Some(128),
        ..Default::default()
    };

    let p2 = RequestProfile::from_request(&req2, "fast");
    let prompt2_len = req2.estimate_prompt_tokens();

    assert_ne!(p1.estimated_prompt_tokens, 0);
    assert_ne!(p2.estimated_prompt_tokens, 0);
    assert_ne!(prompt1_len, prompt2_len);
    assert_eq!(p1.estimated_prompt_tokens, prompt1_len);
    assert_eq!(p2.estimated_prompt_tokens, prompt2_len);
}
