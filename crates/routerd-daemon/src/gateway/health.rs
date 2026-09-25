use crate::rss::MemoryStats;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use routerd_core::RouterEngine;
use serde_json::json;
use std::sync::Arc;

pub async fn health_handler(State(engine): State<Arc<RouterEngine>>) -> impl IntoResponse {
    let status = engine.get_daemon_status().await;
    let mem = MemoryStats::read_current();

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
    });

    (StatusCode::OK, Json(body))
}
