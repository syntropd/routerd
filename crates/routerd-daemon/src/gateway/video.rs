//! Generative Video HTTP gateway endpoint (/v1/video/generations).

use super::codec::images::{base64_encode};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use routerd_core::RouterEngine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::error;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct VideoGenerationRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub frames: Option<usize>,
    pub fps: Option<u32>,
    pub storyboard: Option<usize>,
    pub allow_degrade: Option<bool>,
    pub response_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoObject {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub frames: usize,
    pub fps: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storyboard_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoGenerationResponse {
    pub created: u64,
    pub data: Vec<VideoObject>,
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

async fn call_varlink(sock: &Path, method: &str, params: Value) -> Result<Value, String> {
    let mut stream = UnixStream::connect(sock).await.map_err(|e| e.to_string())?;
    let mut req_bytes = serde_json::to_vec(&json!({ "method": method, "parameters": params })).map_err(|e| e.to_string())?;
    req_bytes.push(0);
    stream.write_all(&req_bytes).await.map_err(|e| e.to_string())?;

    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if n == 0 { return Err("closed prematurely".into()); }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.iter().position(|&b| b == 0) {
            let reply: Value = serde_json::from_slice(&buf[..pos]).map_err(|e| e.to_string())?;
            if let Some(err) = reply.get("error").and_then(|e| e.as_str()) { return Err(err.to_string()); }
            return reply.get("parameters").cloned().ok_or_else(|| "missing params".into());
        }
    }
}

pub async fn video_generations_handler(
    State(_engine): State<Arc<RouterEngine>>,
    Json(request): Json<VideoGenerationRequest>,
) -> Response {
    let prompt = request.prompt.trim();
    if prompt.is_empty() { return bad_request("prompt is required"); }
    let frames = request.frames.unwrap_or(16).clamp(1, 120);
    let fps = request.fps.unwrap_or(8).clamp(1, 60);

    let socket = resolve_runtime_socket();
    let mut params = json!({ "prompt": prompt, "frames": frames, "fps": fps });
    if let Some(sb) = request.storyboard { params["storyboard"] = json!(sb); }
    if let Some(deg) = request.allow_degrade { params["allow_degrade"] = json!(deg); }

    match call_varlink(&socket, "io.syntrop.Runtime1.GenerateVideo", params).await {
        Ok(res) => {
            let path = res.get("video_path").and_then(|v| v.as_str()).unwrap_or("");
            let out_frames = res.get("frames").and_then(|v| v.as_u64()).unwrap_or(frames as u64) as usize;
            let duration_ms = res.get("duration_ms").and_then(|v| v.as_u64());
            let sb_path = res.get("storyboard_path").and_then(|v| v.as_str()).map(|s| format!("file://{s}"));
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            let is_b64 = request.response_format.as_deref() == Some("b64_json");

            let obj = if is_b64 {
                match tokio::fs::read(path).await {
                    Ok(bytes) => VideoObject {
                        b64_json: Some(base64_encode(&bytes)),
                        url: None,
                        frames: out_frames,
                        fps,
                        duration_ms,
                        storyboard_url: sb_path,
                    },
                    Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"message": e.to_string()}}))).into_response(),
                }
            } else {
                VideoObject {
                    b64_json: None,
                    url: Some(format!("file://{path}")),
                    frames: out_frames,
                    fps,
                    duration_ms,
                    storyboard_url: sb_path,
                }
            };

            (StatusCode::OK, Json(VideoGenerationResponse { created: now, data: vec![obj] })).into_response()
        }
        Err(e) => {
            error!("GenerateVideo failed: {e}");
            (StatusCode::BAD_GATEWAY, Json(json!({"error": {"message": e, "type": "runtime_gateway_error"}}))).into_response()
        }
    }
}
