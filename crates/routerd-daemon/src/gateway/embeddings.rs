//! OpenAI-compatible /v1/embeddings HTTP gateway endpoint.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use routerd_core::RouterEngine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::error;

#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingsRequest {
    pub input: Value, // string or array of strings
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingItem {
    pub object: &'static str,
    pub embedding: Vec<f32>,
    pub index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsUsage {
    pub prompt_tokens: usize,
    pub total_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsResponse {
    pub object: &'static str,
    pub data: Vec<EmbeddingItem>,
    pub model: String,
    pub usage: EmbeddingsUsage,
}

fn resolve_runtime_socket() -> PathBuf {
    std::env::var("SYNTROP_RUNTIME_SOCKET")
        .map(|s| PathBuf::from(s.trim()))
        .unwrap_or_else(|_| PathBuf::from("/run/syntrop/io.syntrop.Runtime1"))
}

fn bad_request(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": {"message": msg, "type": "invalid_request_error"}})),
    )
        .into_response()
}

async fn call_varlink_embed(sock: &Path, model: &str, text: &str) -> Result<Vec<f32>, String> {
    let mut stream = UnixStream::connect(sock).await.map_err(|e| e.to_string())?;
    let req_bytes = serde_json::to_vec(&json!({
        "method": "io.syntrop.Runtime1.Embed",
        "parameters": { "model": model, "text": text },
    })).map_err(|e| e.to_string())?;
    let mut payload = req_bytes;
    payload.push(0);
    stream.write_all(&payload).await.map_err(|e| e.to_string())?;

    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if n == 0 { return Err("closed prematurely".into()); }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.iter().position(|&b| b == 0) {
            let reply: Value = serde_json::from_slice(&buf[..pos]).map_err(|e| e.to_string())?;
            if let Some(err) = reply.get("error").and_then(|e| e.as_str()) { return Err(err.to_string()); }
            let params = reply.get("parameters").ok_or_else(|| "missing params".to_string())?;
            let emb_val = params.get("embedding").ok_or_else(|| "missing embedding".to_string())?;
            let floats: Vec<f32> = serde_json::from_value(emb_val.clone()).map_err(|e| e.to_string())?;
            return Ok(floats);
        }
    }
}

pub async fn embeddings_handler(
    State(_engine): State<Arc<RouterEngine>>,
    Json(request): Json<EmbeddingsRequest>,
) -> Response {
    let texts: Vec<String> = match request.input {
        Value::String(s) if !s.trim().is_empty() => vec![s],
        Value::Array(arr) => {
            let mut list = Vec::with_capacity(arr.len());
            for v in arr {
                if let Some(s) = v.as_str() {
                    list.push(s.to_string());
                }
            }
            if list.is_empty() { return bad_request("input array must contain non-empty strings"); }
            list
        }
        _ => return bad_request("input must be a non-empty string or array of strings"),
    };

    let model = request.model.unwrap_or_else(|| "default-embed".to_string());
    let socket = resolve_runtime_socket();
    let mut data = Vec::with_capacity(texts.len());
    let mut total_tokens = 0;

    for (index, text) in texts.iter().enumerate() {
        total_tokens += text.split_whitespace().count();
        match call_varlink_embed(&socket, &model, text).await {
            Ok(embedding) => data.push(EmbeddingItem { object: "embedding", embedding, index }),
            Err(e) => {
                error!("Embed call failed: {e}");
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": {"message": e, "type": "runtime_gateway_error"}})),
                ).into_response();
            }
        }
    }

    (
        StatusCode::OK,
        Json(EmbeddingsResponse {
            object: "list",
            data,
            model,
            usage: EmbeddingsUsage { prompt_tokens: total_tokens, total_tokens },
        }),
    ).into_response()
}
