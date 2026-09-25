use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use routerd_core::{ChatCompletionRequest, RouterEngine, RouterError};
use serde_json::json;
use std::sync::Arc;
use tracing::{debug, error};

pub async fn chat_completions_handler(
    State(engine): State<Arc<RouterEngine>>,
    headers: HeaderMap,
    Json(mut request): Json<ChatCompletionRequest>,
) -> Response {
    // If request tier is not set in body, check header 'x-syntrop-tier'
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

                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .header(header::CACHE_CONTROL, "no-cache")
                    .header(header::CONNECTION, "keep-alive")
                    .header("x-syntrop-provider", scored.provider_id)
                    .header("x-syntrop-model", scored.model_name)
                    .body(Body::from_stream(stream))
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
            Ok((response, scored)) => {
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
