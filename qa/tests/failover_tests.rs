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

async fn mock_chat_completion(
    Json(_payload): Json<Value>,
) -> Json<ChatCompletionResponse> {
    Json(ChatCompletionResponse {
        id: "chatcmpl-mock-backup".to_string(),
        object: "chat.completion".to_string(),
        created: 1727250000,
        model: "backup-model".to_string(),
        choices: vec![ChatChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_string(),
                content: serde_json::json!("Failover response from backup provider"),
                name: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: None,
    })
}

#[tokio::test]
async fn test_provider_failover_retry_success() {
    // 1. Spawn a local mock HTTP server that represents the healthy backup provider
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr: SocketAddr = listener.local_addr().unwrap();

    let app = Router::new().route("/v1/chat/completions", post(mock_chat_completion));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // 2. Configure two providers:
    //    Provider A ("primary-failing"): unreachable port 1 -> higher score (ranked 1st)
    //    Provider B ("backup-healthy"): reachable local server -> lower score (ranked 2nd)
    let mut config = RouterConfig::default();
    config.thresholds.max_retries = 2;

    config.providers.push(ProviderConfig {
        id: "primary-failing".to_string(),
        name: "Primary Failing Provider".to_string(),
        kind: "openai".to_string(),
        base_url: "http://127.0.0.1:1/v1".to_string(), // Unreachable port
        api_key: None,
        tier: "fast".to_string(),
        weight: 1.5, // Higher weight puts it first in scoring
        enabled: true,
        timeout_ms: 500,
        models: vec![ProviderModelConfig {
            name: "fast-model-primary".to_string(),
            max_context_tokens: 4096,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            avg_latency_ms: 10.0,
            tokens_per_second: 300.0,
            tier: Some("fast".to_string()),
        }],
    });

    config.providers.push(ProviderConfig {
        id: "backup-healthy".to_string(),
        name: "Backup Healthy Provider".to_string(),
        kind: "openai".to_string(),
        base_url: format!("http://{}/v1", local_addr),
        api_key: None,
        tier: "fast".to_string(),
        weight: 1.0, // Lower weight puts it second
        enabled: true,
        timeout_ms: 2000,
        models: vec![ProviderModelConfig {
            name: "backup-model".to_string(),
            max_context_tokens: 4096,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            avg_latency_ms: 150.0,
            tokens_per_second: 100.0,
            tier: Some("fast".to_string()),
        }],
    });

    let engine = Arc::new(RouterEngine::new(config));

    let req = ChatCompletionRequest {
        model: "router:fast".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: serde_json::json!("Test failover query"),
            name: None,
        }],
        temperature: None,
        top_p: None,
        max_tokens: Some(64),
        max_completion_tokens: None,
        stream: Some(false),
        tier: Some("fast".to_string()),
        extra: std::collections::HashMap::new(),
    };

    // 3. Dispatch chat completion. It should attempt primary, fail, and gracefully failover to backup
    let result = engine.route_chat(&req).await;
    assert!(result.is_ok(), "Expected failover to succeed, got: {:?}", result.err());

    let (response, scored) = result.unwrap();
    assert_eq!(scored.provider_id, "backup-healthy");
    assert_eq!(scored.model_name, "backup-model");
    assert_eq!(response.id, "chatcmpl-mock-backup");

    // 4. Verify telemetry stats recorded the failure on primary and success on backup
    let statuses = engine.list_provider_statuses().await;
    let primary_status = statuses.iter().find(|p| p.id == "primary-failing").unwrap();
    let backup_status = statuses.iter().find(|p| p.id == "backup-healthy").unwrap();

    assert_eq!(primary_status.total_errors, 1);
    assert_eq!(primary_status.total_requests, 0);

    assert_eq!(backup_status.total_errors, 0);
    assert_eq!(backup_status.total_requests, 1);
    assert!(backup_status.is_healthy);
}

#[tokio::test]
async fn test_provider_failover_exhaustion() {
    let mut config = RouterConfig::default();
    config.thresholds.max_retries = 1;

    config.providers.push(ProviderConfig {
        id: "dead-1".to_string(),
        name: "Dead Provider 1".to_string(),
        kind: "openai".to_string(),
        base_url: "http://127.0.0.1:1/v1".to_string(),
        api_key: None,
        tier: "fast".to_string(),
        weight: 1.2,
        enabled: true,
        timeout_ms: 300,
        models: vec![ProviderModelConfig {
            name: "model-dead-1".to_string(),
            max_context_tokens: 4096,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            avg_latency_ms: 10.0,
            tokens_per_second: 300.0,
            tier: Some("fast".to_string()),
        }],
    });

    config.providers.push(ProviderConfig {
        id: "dead-2".to_string(),
        name: "Dead Provider 2".to_string(),
        kind: "openai".to_string(),
        base_url: "http://127.0.0.1:2/v1".to_string(),
        api_key: None,
        tier: "fast".to_string(),
        weight: 1.0,
        enabled: true,
        timeout_ms: 300,
        models: vec![ProviderModelConfig {
            name: "model-dead-2".to_string(),
            max_context_tokens: 4096,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            avg_latency_ms: 20.0,
            tokens_per_second: 300.0,
            tier: Some("fast".to_string()),
        }],
    });

    let engine = Arc::new(RouterEngine::new(config));

    let req = ChatCompletionRequest {
        model: "router:fast".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: serde_json::json!("Test failover exhaustion"),
            name: None,
        }],
        temperature: None,
        top_p: None,
        max_tokens: Some(64),
        max_completion_tokens: None,
        stream: Some(false),
        tier: Some("fast".to_string()),
        extra: std::collections::HashMap::new(),
    };

    let result = engine.route_chat(&req).await;
    assert!(result.is_err(), "Expected error when all providers fail");
}
