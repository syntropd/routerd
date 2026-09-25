pub mod chat;
pub mod health;
pub mod models;

use axum::routing::{get, post};
use axum::Router;
use routerd_core::RouterEngine;
use std::sync::Arc;

pub fn create_gateway_router(engine: Arc<RouterEngine>) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(chat::chat_completions_handler))
        .route("/v1/models", get(models::models_handler))
        .route("/health", get(health::health_handler))
        .with_state(engine)
}
