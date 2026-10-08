//! Universal Standard Gateway HTTP Router for syntropd.
//!
//! Exposes standard OpenAI-compatible endpoints across all multimodal engines:
//! text, images, speech, audio/music, video, embeddings, and reflex decisions,
//! while reverse-proxying non-API studio/gallery traffic to the local studio worker.

pub mod audio;
pub mod chat;
pub mod codec;
pub mod embeddings;
pub mod images;
pub mod systemone;
pub mod video;

use crate::rss::MemoryStats;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use routerd_core::{ModelListResponse, RouterEngine};
use serde_json::json;
use std::sync::Arc;

pub fn create_gateway_router(engine: Arc<RouterEngine>) -> Router {
    Router::new()
        // Text LLM (Chat & Legacy Completion)
        .route("/v1/chat/completions", post(chat::chat_completions_handler))
        .route("/v1/completions", post(chat::completions_handler))
        // Embeddings
        .route("/v1/embeddings", post(embeddings::embeddings_handler))
        // Visual (Generative Images & Inpainting)
        .route("/v1/images/generations", post(images::image_generations_handler))
        .route("/v1/images/edits", post(images::image_edits_handler))
        // Speech & Audio Transcription / TTS / Generative Music
        .route("/v1/audio/transcriptions", post(audio::audio_transcriptions_handler))
        .route("/v1/audio/speech", post(audio::audio_speech_handler))
        .route("/v1/audio/generations", post(audio::audio_generations_handler))
        .route("/v1/audio/music", post(audio::audio_generations_handler))
        // Video Generation
        .route("/v1/video/generations", post(video::video_generations_handler))
        .route("/v1/videos/generations", post(video::video_generations_handler))
        // Fast Reflexive Decisions & Catalog
        .route("/v1/systemone", post(systemone::systemone_handler))
        .route("/v1/models", get(models_handler))
        .route("/health", get(health_handler))
        .fallback(fallback_studio_proxy)
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

pub async fn fallback_studio_proxy(req: Request) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let target_url = format!("http://127.0.0.1:19820{path_and_query}");

    let client = reqwest::Client::new();
    let mut builder = client.request(method, &target_url);
    for (k, v) in req.headers() {
        if k != "host" {
            builder = builder.header(k, v);
        }
    }
    let body_bytes = match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes);
    }

    match builder.send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
            let mut response_builder = Response::builder().status(status);
            for (k, v) in resp.headers() {
                response_builder = response_builder.header(k, v);
            }
            let bytes = resp.bytes().await.unwrap_or_default();
            response_builder
                .body(axum::body::Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(_) => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}
