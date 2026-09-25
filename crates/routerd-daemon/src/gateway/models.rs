use axum::extract::State;
use axum::Json;
use routerd_core::{ModelListResponse, RouterEngine};
use std::sync::Arc;

pub async fn models_handler(State(engine): State<Arc<RouterEngine>>) -> Json<ModelListResponse> {
    let models = engine.list_all_models().await;
    Json(models)
}
