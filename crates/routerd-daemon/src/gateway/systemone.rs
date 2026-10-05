//! SystemOne decision HTTP gateway endpoint (/v1/systemone).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use routerd_core::RouterEngine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::{debug, error};

/// HTTP request body for /v1/systemone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemOneHttpRequest {
    #[serde(default = "default_decision_model")]
    pub model: String,
    pub state: String,
    pub questions: Map<String, Value>,
}

fn default_decision_model() -> String {
    "clef-flash".to_string()
}

fn resolve_runtime_socket() -> PathBuf {
    std::env::var("SYNTROP_RUNTIME_SOCKET")
        .map(|s| PathBuf::from(s.trim()))
        .unwrap_or_else(|_| PathBuf::from("/run/syntrop/io.syntrop.Runtime1"))
}

/// Handler for POST /v1/systemone.
pub async fn systemone_handler(
    State(_engine): State<Arc<RouterEngine>>,
    Json(payload): Json<SystemOneHttpRequest>,
) -> Response {
    let socket = resolve_runtime_socket();
    let params = json!({
        "model": payload.model,
        "state": payload.state,
        "questions": payload.questions,
    });

    debug!("Calling io.syntrop.Decision1.Decide over Varlink socket {:?}", socket);
    match call_varlink_decision(&socket, "io.syntrop.Decision1.Decide", params).await {
        Ok(res) => (StatusCode::OK, Json(res)).into_response(),
        Err(e) => {
            error!("SystemOne decision call failed: {}", e);
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": {
                        "message": e,
                        "type": "decision_error",
                    }
                })),
            )
                .into_response()
        }
    }
}

async fn call_varlink_decision(sock: &Path, method: &str, params: Value) -> Result<Value, String> {
    let mut stream = UnixStream::connect(sock)
        .await
        .map_err(|e| format!("connect to runtimed failed: {e}"))?;

    let mut req_bytes = serde_json::to_vec(&json!({
        "method": method,
        "parameters": params,
    }))
    .map_err(|e| e.to_string())?;
    req_bytes.push(0);

    stream
        .write_all(&req_bytes)
        .await
        .map_err(|e| e.to_string())?;

    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("closed prematurely".into());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.iter().position(|&b| b == 0) {
            let slice = &buf[..pos];
            let resp: Value = serde_json::from_slice(slice).map_err(|e| e.to_string())?;
            if let Some(err) = resp.get("error").and_then(|v| v.as_str()) {
                return Err(format!("Varlink error: {err}"));
            }
            if let Some(parameters) = resp.get("parameters") {
                return Ok(parameters.clone());
            }
            return Ok(resp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_systemone_http_request_default_model() {
        let raw = json!({
            "state": "memory is low",
            "questions": {
                "check": { "type": "bool", "instructions": "check mem" }
            }
        });
        let req: SystemOneHttpRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.model, "clef-flash");
        assert_eq!(req.state, "memory is low");
    }

    #[test]
    fn test_systemone_http_request_custom_model() {
        let raw = json!({
            "model": "clef",
            "state": "incident report",
            "questions": {}
        });
        let req: SystemOneHttpRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.model, "clef");
    }
}
