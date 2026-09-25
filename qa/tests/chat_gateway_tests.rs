use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use routerd_core::{ProviderConfig, ProviderModelConfig, RouterConfig, RouterEngine};
use routerd_daemon::gateway::create_gateway_router;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

fn test_router_engine() -> Arc<RouterEngine> {
    let mut config = RouterConfig::default();
    config.providers.push(ProviderConfig {
        id: "mock-fast".to_string(),
        name: "Mock Fast Provider".to_string(),
        kind: "openai".to_string(),
        base_url: "http://127.0.0.1:9".to_string(),
        api_key: None,
        tier: "fast".to_string(),
        weight: 1.0,
        enabled: true,
        timeout_ms: 1000,
        models: vec![ProviderModelConfig {
            name: "test-fast-model".to_string(),
            max_context_tokens: 4096,
            cost_per_input_token: 0.0000001,
            cost_per_output_token: 0.0000002,
            avg_latency_ms: 50.0,
            tokens_per_second: 200.0,
            tier: Some("fast".to_string()),
        }],
    });
    Arc::new(RouterEngine::new(config))
}

#[tokio::test]
async fn test_health_endpoint() {
    let engine = test_router_engine();
    let app = create_gateway_router(engine);

    let req = Request::builder()
        .uri("/health")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json.get("status").unwrap(), "active");
    assert!(json.get("rss_mb").is_some());
    assert!(json.get("psi_level").is_some());
}

#[tokio::test]
async fn test_models_endpoint() {
    let engine = test_router_engine();
    let app = create_gateway_router(engine);

    let req = Request::builder()
        .uri("/v1/models")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let data = json.get("data").unwrap().as_array().unwrap();
    let model_ids: Vec<&str> = data.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str())).collect();

    assert!(model_ids.contains(&"router:fast"));
    assert!(model_ids.contains(&"test-fast-model"));
}

#[tokio::test]
async fn test_chat_completions_context_limit_exceeded_returns_bad_request() {
    let engine = test_router_engine();
    let app = create_gateway_router(engine);

    // Context limit for mock-fast is 4096 tokens. We request 8192 max_tokens.
    let payload = json!({
        "model": "router:fast",
        "messages": [
            { "role": "user", "content": "Hello" }
        ],
        "max_tokens": 8192
    });

    let req = Request::builder()
        .uri("/v1/chat/completions")
        .method("POST")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let err_type = json.get("error").unwrap().get("type").unwrap().as_str().unwrap();
    assert_eq!(err_type, "context_limit_exceeded");
}
