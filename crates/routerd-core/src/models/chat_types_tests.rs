use super::*;
use serde_json::Value;

fn request(model: &str, tier: Option<&str>, max_tokens: Option<usize>) -> ChatCompletionRequest {
    ChatCompletionRequest {
        model: model.to_string(),
        messages: vec![ChatMessage::new("user", serde_json::json!("hello"))],
        max_tokens,
        tier: tier.map(str::to_string),
        ..Default::default()
    }
}

#[test]
fn content_as_str_reads_all_shapes() {
    let plain = ChatMessage::new("r", serde_json::json!("hi"));
    assert_eq!(plain.content_as_str(), "hi");
    let parts = ChatMessage::new(
        "r",
        serde_json::json!([{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]),
    );
    assert_eq!(parts.content_as_str(), "a b ");
    let null_msg = ChatMessage::new("assistant", Value::Null);
    assert_eq!(null_msg.content_as_str(), "");
}

#[test]
fn requested_tier_prefers_field_then_alias() {
    assert_eq!(request("x", Some("hard"), None).requested_tier(), Some("hard"));
    assert_eq!(request("router:fast", None, None).requested_tier(), Some("fast"));
    assert_eq!(request("hard", None, None).requested_tier(), Some("hard"));
    assert_eq!(request("llama", None, None).requested_tier(), None);
}

#[test]
fn token_estimates_floor_at_one_and_add_output() {
    let mut req = request("router:fast", None, Some(100));
    req.messages.clear();
    assert_eq!(req.estimate_prompt_tokens(), 1);
    assert_eq!(req.estimate_total_tokens(), 101);
    let req = request("router:fast", None, None);
    assert!(req.estimate_prompt_tokens() >= 1);
    assert_eq!(req.estimate_total_tokens(), req.estimate_prompt_tokens() + 2048);
}

#[test]
fn reasoning_budget_prefers_explicit_then_max_thinking() {
    let mut req = request("test", None, None);
    assert_eq!(req.reasoning_budget(), None);
    req.max_thinking_tokens = Some(500);
    assert_eq!(req.reasoning_budget(), Some(500));
    req.reasoning_budget = Some(1000);
    assert_eq!(req.reasoning_budget(), Some(1000));
}

#[test]
fn extract_image_base64_from_array_and_data_url() {
    let msg = ChatMessage::new(
        "user",
        serde_json::json!([
            {"type": "text", "text": "Describe this image"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg=="}}
        ]),
    );
    assert_eq!(
        msg.extract_image_base64(),
        Some("iVBORw0KGgoAAAANSUhEUg==".to_string())
    );

    let mut req = request("model", None, None);
    req.messages = vec![msg];
    assert_eq!(
        req.extract_image_base64(),
        Some("iVBORw0KGgoAAAANSUhEUg==".to_string())
    );
}
