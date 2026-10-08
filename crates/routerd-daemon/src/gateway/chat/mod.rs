//! OpenAI-compatible /v1/chat/completions and legacy /v1/completions gateway endpoints.

pub mod sse_transform;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use routerd_core::models::ChatMessage;
use routerd_core::{ChatCompletionRequest, RouterEngine, RouterError};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tracing::{debug, error};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LegacyCompletionRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub max_tokens: Option<usize>,
    pub temperature: Option<f32>,
    pub stream: Option<bool>,
}

pub async fn chat_completions_handler(
    State(engine): State<Arc<RouterEngine>>,
    headers: HeaderMap,
    Json(mut request): Json<ChatCompletionRequest>,
) -> Response {
    if request.tier.is_none() {
        if let Some(tier_hdr) = headers.get("x-syntrop-tier").and_then(|v| v.to_str().ok()) {
            request.tier = Some(tier_hdr.to_string());
        }
    }

    let is_stream = request.stream.unwrap_or(false);

    if is_stream {
        match engine.route_chat_stream(&request).await {
            Ok((stream, scored)) => {
                debug!(
                    "Streaming response routed to provider '{}' (model '{}')",
                    scored.provider_id, scored.model_name
                );
                let transformed = sse_transform::transform_sse_stream(stream);

                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .header(header::CACHE_CONTROL, "no-cache")
                    .header(header::CONNECTION, "keep-alive")
                    .header("x-syntrop-provider", scored.provider_id)
                    .header("x-syntrop-model", scored.model_name)
                    .body(Body::from_stream(transformed))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
            }
            Err(e) => {
                error!("Routing streaming chat completion failed: {}", e);
                let (status, err_type) = map_error_status(&e);
                (
                    status,
                    Json(json!({
                        "error": {
                            "message": e.to_string(),
                            "type": err_type,
                            "param": null,
                            "code": null
                        }
                    })),
                )
                    .into_response()
            }
        }
    } else {
        match engine.route_chat(&request).await {
            Ok((mut response, scored)) => {
                sse_transform::clean_completion_response(&mut response);
                debug!(
                    "Non-streaming response routed to provider '{}' (model '{}')",
                    scored.provider_id, scored.model_name
                );

                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("x-syntrop-provider", scored.provider_id)
                    .header("x-syntrop-model", scored.model_name)
                    .body(Body::from(serde_json::to_vec(&response).unwrap_or_default()))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
            }
            Err(e) => {
                error!("Routing chat completion failed: {}", e);
                let (status, err_type) = map_error_status(&e);
                (
                    status,
                    Json(json!({
                        "error": {
                            "message": e.to_string(),
                            "type": err_type,
                            "param": null,
                            "code": null
                        }
                    })),
                )
                    .into_response()
            }
        }
    }
}

pub async fn completions_handler(
    state: State<Arc<RouterEngine>>,
    headers: HeaderMap,
    Json(legacy): Json<LegacyCompletionRequest>,
) -> Response {
    let chat_req = ChatCompletionRequest {
        model: legacy.model.unwrap_or_else(|| "default".to_string()),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: serde_json::Value::String(legacy.prompt),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        }],
        max_tokens: legacy.max_tokens,
        temperature: legacy.temperature,
        stream: legacy.stream,
        ..Default::default()
    };
    chat_completions_handler(state, headers, Json(chat_req)).await
}

fn map_error_status(err: &RouterError) -> (StatusCode, &'static str) {
    match err {
        RouterError::ContextLimitExceeded { .. } => {
            (StatusCode::BAD_REQUEST, "context_limit_exceeded")
        }
        RouterError::NoHealthyProvider(_) => {
            (StatusCode::SERVICE_UNAVAILABLE, "no_healthy_provider")
        }
        RouterError::ProviderUnavailable { .. } => {
            (StatusCode::BAD_GATEWAY, "provider_unavailable")
        }
        RouterError::Timeout(_) => (StatusCode::GATEWAY_TIMEOUT, "timeout"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
    }
}
