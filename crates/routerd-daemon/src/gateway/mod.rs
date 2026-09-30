pub mod chat;
pub mod health;
pub mod sse_transform;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use routerd_core::{ModelListResponse, RouterEngine};
use std::sync::Arc;

pub fn create_gateway_router(engine: Arc<RouterEngine>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(chat::chat_completions_handler))
        .route("/v1/models", get(models_handler))
        .route("/health", get(health::health_handler))
        .with_state(engine)
}

pub async fn models_handler(State(engine): State<Arc<RouterEngine>>) -> Json<ModelListResponse> {
    let models = engine.list_all_models().await;
    Json(models)
}
