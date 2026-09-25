use super::protocol::VarlinkReply;
use routerd_core::RouterEngine;
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn handle_method(
    method: &str,
    params: Option<&Value>,
    engine: &Arc<RouterEngine>,
) -> Option<VarlinkReply> {
    let sub = method.strip_prefix("io.syntrop.Router1.")?;

    match sub {
        "GetStatus" => Some(handle_get_status(engine).await),
        "ListProviders" => Some(handle_list_providers(engine).await),
        "ListModels" => Some(handle_list_models(engine).await),
        "RouteRequest" => Some(handle_route_request(params, engine).await),
        "TestProvider" => Some(handle_test_provider(params, engine).await),
        _ => None,
    }
}

async fn handle_get_status(engine: &Arc<RouterEngine>) -> VarlinkReply {
    let status = engine.get_daemon_status().await;
    VarlinkReply::ok(json!({
        "status": status.status,
        "version": status.version,
        "uptime_seconds": status.uptime_seconds,
        "total_requests": status.total_requests,
        "active_requests": status.active_requests,
        "providers_count": status.providers_count,
        "healthy_providers_count": status.healthy_providers_count,
        "psi_level": status.psi_level,
        "psi_memory_some": status.psi_memory_some,
        "rss_bytes": status.rss_bytes,
        "rss_mb": status.rss_mb
    }))
}

async fn handle_list_providers(engine: &Arc<RouterEngine>) -> VarlinkReply {
    let providers = engine.list_provider_statuses().await;
    VarlinkReply::ok(json!({ "providers": providers }))
}

async fn handle_list_models(engine: &Arc<RouterEngine>) -> VarlinkReply {
    let models = engine.list_all_models().await;
    let model_names: Vec<String> = models.data.into_iter().map(|m| m.id).collect();
    VarlinkReply::ok(json!({ "models": model_names }))
}

async fn handle_route_request(params: Option<&Value>, engine: &Arc<RouterEngine>) -> VarlinkReply {
    let model = params.and_then(|p| p.get("model")).and_then(|v| v.as_str());
    let tier = params.and_then(|p| p.get("tier")).and_then(|v| v.as_str());
    let tokens = params
        .and_then(|p| p.get("estimated_tokens"))
        .and_then(|v| v.as_u64())
        .map(|u| u as usize);
    let require_stream = params
        .and_then(|p| p.get("require_stream"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match engine.simulate_route(model, tier, tokens, require_stream).await {
        Ok(candidates) => VarlinkReply::ok(json!({ "candidates": candidates })),
        Err(e) => VarlinkReply::error(
            "io.syntrop.Router1.RoutingFailed",
            json!({ "message": e.to_string() }),
        ),
    }
}

async fn handle_test_provider(params: Option<&Value>, engine: &Arc<RouterEngine>) -> VarlinkReply {
    let provider_id = match params.and_then(|p| p.get("provider_id")).and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return VarlinkReply::error(
                "org.varlink.service.InvalidParameter",
                json!({ "parameter": "provider_id" }),
            )
        }
    };

    match engine.test_provider(provider_id).await {
        Ok((healthy, latency, err)) => VarlinkReply::ok(json!({
            "provider_id": provider_id,
            "healthy": healthy,
            "latency_ms": latency,
            "error": err
        })),
        Err(e) => VarlinkReply::error(
            "io.syntrop.Router1.TestFailed",
            json!({ "message": e.to_string() }),
        ),
    }
}
