//! Integration tests for Cascade routing and elastic step-down in routerd-core.

use axum::extract::Json;
use axum::routing::post;
use axum::Router;
use routerd_core::{
    ChatChoice, ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ProviderConfig,
    ProviderModelConfig, RouterConfig, RouterEngine,
};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::const_new(());

async fn mock_cascade_completion(
    Json(payload): Json<Value>,
) -> Json<ChatCompletionResponse> {
    let model = payload.get("model").and_then(|m| m.as_str()).unwrap_or("unknown");
    Json(ChatCompletionResponse {
        id: format!("chatcmpl-mock-{model}"),
        object: "chat.completion".to_string(),
        created: 1727250000,
        model: model.to_string(),
        choices: vec![ChatChoice {
            index: 0,
            message: ChatMessage::new("assistant", serde_json::json!("Response")),
            tool_calls: None,
            finish_reason: Some("stop".to_string()),
        }],
        usage: None,
    })
}

fn build_cascade_config(base_url: &str) -> RouterConfig {
    let mut config = RouterConfig::default();
    config.daemon.inferenced_socket = "/tmp/qa-test-inferenced-never.sock".into();
    config.thresholds.max_retries = 1;
    config.providers.push(ProviderConfig {
        id: "local-node".to_string(),
        name: "Local Combined Engine".to_string(),
        kind: "openai".to_string(),
        base_url: format!("{base_url}/v1"),
        api_key: None,
        tier: "fast".to_string(),
        weight: 1.0,
        enabled: true,
        timeout_ms: 1000,
        models: vec![
            ProviderModelConfig {
                name: "qwen2.5-0.5b".to_string(),
                max_context_tokens: 32768,
                cost_per_input_token: 0.0,
                cost_per_output_token: 0.0,
                avg_latency_ms: 15.0,
                tokens_per_second: 150.0,
                tier: Some("fast".to_string()),
                draft_model: None,
            },
            ProviderModelConfig {
                name: "qwen2.5-1.5b".to_string(),
                max_context_tokens: 32768,
                cost_per_input_token: 0.0,
                cost_per_output_token: 0.0,
                avg_latency_ms: 30.0,
                tokens_per_second: 100.0,
                tier: Some("fast".to_string()),
                draft_model: None,
            },
            ProviderModelConfig {
                name: "qwen2.5-7b".to_string(),
                max_context_tokens: 32768,
                cost_per_input_token: 0.0,
                cost_per_output_token: 0.0,
                avg_latency_ms: 60.0,
                tokens_per_second: 60.0,
                tier: Some("fast".to_string()),
                draft_model: None,
            },
        ],
    });
    config
}

#[tokio::test]
async fn test_cascade_routes_simple_query_to_cpu_draft() {
    let _lock = ENV_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr: SocketAddr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/chat/completions", post(mock_cascade_completion));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let config = build_cascade_config(&format!("http://{local_addr}"));
    let engine = Arc::new(RouterEngine::new(config));

    let req = ChatCompletionRequest {
        model: "qwen2.5-7b".to_string(),
        messages: vec![ChatMessage::new("user", serde_json::json!("Hello, how are you?"))],
        max_tokens: Some(32),
        stream: Some(false),
        tier: Some("fast".to_string()),
        ..Default::default()
    };

    let result = engine.route_chat(&req).await;
    assert!(result.is_ok(), "Expected routing to succeed, got: {:?}", result.err());

    let (response, scored) = result.unwrap();
    assert_eq!(scored.model_name, "qwen2.5-0.5b", "Reflexive query must route to CPU draft model");
    assert_eq!(response.model, "qwen2.5-0.5b");
}

#[tokio::test]
async fn test_cascade_retains_gpu_model_for_complex_query() {
    let _lock = ENV_LOCK.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr: SocketAddr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/chat/completions", post(mock_cascade_completion));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let config = build_cascade_config(&format!("http://{local_addr}"));
    let engine = Arc::new(RouterEngine::new(config));

    let req = ChatCompletionRequest {
        model: "qwen2.5-7b".to_string(),
        messages: vec![ChatMessage::new(
            "user",
            serde_json::json!("```rust\nfn main() { println!(\"deep reasoning\"); }\n```"),
        )],
        max_tokens: Some(32),
        stream: Some(false),
        tier: Some("fast".to_string()),
        ..Default::default()
    };

    let result = engine.route_chat(&req).await;
    assert!(result.is_ok(), "Expected routing to succeed, got: {:?}", result.err());

    let (response, scored) = result.unwrap();
    assert_eq!(scored.model_name, "qwen2.5-7b", "Complex code query must remain on System 2 GPU model");
    assert_eq!(response.model, "qwen2.5-7b");
}

#[tokio::test]
async fn test_elastic_downgrade_under_elevated_pressure() {
    let _lock = ENV_LOCK.lock().await;
    std::env::set_var("INFERENCED_SIMULATE_PSI", "elevated");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr: SocketAddr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/chat/completions", post(mock_cascade_completion));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let config = build_cascade_config(&format!("http://{local_addr}"));
    let engine = Arc::new(RouterEngine::new(config));

    let req = ChatCompletionRequest {
        model: "qwen2.5-7b".to_string(),
        messages: vec![ChatMessage::new(
            "user",
            serde_json::json!("```rust\nfn main() { println!(\"code\"); }\n```"),
        )],
        max_tokens: Some(32),
        stream: Some(false),
        tier: Some("fast".to_string()),
        ..Default::default()
    };

    let result = engine.route_chat(&req).await;
    std::env::remove_var("INFERENCED_SIMULATE_PSI");

    assert!(result.is_ok(), "Routing failed: {:?}", result.err());
    let (response, scored) = result.unwrap();
    assert_eq!(scored.model_name, "qwen2.5-1.5b", "Elevated pressure must step 7B down to 1.5B");
    assert_eq!(response.model, "qwen2.5-1.5b");
}

#[tokio::test]
async fn test_elastic_downgrade_under_critical_pressure() {
    let _lock = ENV_LOCK.lock().await;
    std::env::set_var("INFERENCED_SIMULATE_PSI", "critical");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr: SocketAddr = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/chat/completions", post(mock_cascade_completion));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let config = build_cascade_config(&format!("http://{local_addr}"));
    let engine = Arc::new(RouterEngine::new(config));

    let req = ChatCompletionRequest {
        model: "qwen2.5-7b".to_string(),
        messages: vec![ChatMessage::new(
            "user",
            serde_json::json!("```rust\nfn main() { println!(\"code\"); }\n```"),
        )],
        max_tokens: Some(32),
        stream: Some(false),
        tier: Some("fast".to_string()),
        ..Default::default()
    };

    let result = engine.route_chat(&req).await;
    std::env::remove_var("INFERENCED_SIMULATE_PSI");

    assert!(result.is_ok(), "Routing failed: {:?}", result.err());
    let (response, scored) = result.unwrap();
    assert_eq!(scored.model_name, "qwen2.5-0.5b", "Critical pressure must step down to 0.5B");
    assert_eq!(response.model, "qwen2.5-0.5b");
}
