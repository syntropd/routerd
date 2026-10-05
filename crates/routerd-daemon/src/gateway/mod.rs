pub mod audio;
pub mod audio_codec;
pub mod chat;
pub mod images;
pub mod images_codec;
pub mod sse_transform;
pub mod systemone;

use crate::rss::MemoryStats;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use routerd_core::{ModelListResponse, RouterEngine};
use serde_json::json;
use std::sync::Arc;

pub fn create_gateway_router(engine: Arc<RouterEngine>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(chat::chat_completions_handler))
        .route(
            "/v1/images/generations",
            post(images::image_generations_handler),
        )
        .route("/v1/images/edits", post(images::image_edits_handler))
        .route(
            "/v1/audio/transcriptions",
            post(audio::audio_transcriptions_handler),
        )
        .route("/v1/audio/speech", post(audio::audio_speech_handler))
        .route("/v1/systemone", post(systemone::systemone_handler))
        .route("/v1/models", get(models_handler))
        .route("/health", get(health_handler))
        .with_state(engine)
}

pub async fn models_handler(State(engine): State<Arc<RouterEngine>>) -> Json<ModelListResponse> {
    let models = engine.list_all_models().await;
    Json(models)
}

pub async fn health_handler(State(engine): State<Arc<RouterEngine>>) -> impl IntoResponse {
    let status = engine.get_daemon_status().await;
    let mem = MemoryStats::read_current();
    let hw = engine.get_hardware_telemetry().await;

    let body = json!({
        "status": status.status,
        "version": status.version,
        "uptime_seconds": status.uptime_seconds,
        "total_requests": status.total_requests,
        "active_requests": status.active_requests,
        "providers_count": status.providers_count,
        "healthy_providers_count": status.healthy_providers_count,
        "psi_level": status.psi_level,
        "psi_memory_some": status.psi_memory_some,
        "rss_bytes": mem.rss_bytes,
        "rss_mb": mem.rss_mb(),
        "telemetry": hw,
    });

    (StatusCode::OK, Json(body))
}
