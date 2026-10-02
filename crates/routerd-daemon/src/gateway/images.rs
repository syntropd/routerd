//! OpenAI-compatible /v1/images HTTP gateway endpoints.

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
use tracing::{debug, error};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImageGenerationRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub n: Option<usize>,
    pub size: Option<String>,
    pub quality: Option<String>,
    pub response_format: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImageEditRequest {
    pub prompt: Option<String>,
    pub image: Option<String>,
    pub mask: Option<String>,
    pub model: Option<String>,
    pub n: Option<usize>,
    pub size: Option<String>,
    pub response_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageObject {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerationResponse {
    pub created: u64,
    pub data: Vec<ImageObject>,
}

pub use super::images_codec::{base64_decode, base64_encode, parse_dimensions};

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

async fn call_generate_visual(
    sock: &Path,
    prompt: &str,
    model: Option<&str>,
    width: u32,
    height: u32,
    seed: u64,
) -> Result<Value, String> {
    let mut stream = UnixStream::connect(sock)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    let mut params = json!({ "prompt": prompt, "width": width, "height": height, "seed": seed });
    if let Some(m) = model {
        params["model"] = json!(m);
    }
    let mut req_bytes = serde_json::to_vec(&json!({
        "method": "io.syntrop.Runtime1.GenerateVisual",
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
            let reply: Value = serde_json::from_slice(&buf[..pos]).map_err(|e| e.to_string())?;
            if let Some(err) = reply.get("error").and_then(|e| e.as_str()) {
                return Err(err.to_string());
            }
            return reply
                .get("parameters")
                .cloned()
                .ok_or_else(|| "missing params".into());
        }
    }
}

pub async fn image_generations_handler(
    State(_engine): State<Arc<RouterEngine>>,
    Json(request): Json<ImageGenerationRequest>,
) -> Response {
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        return bad_request("prompt is required");
    }

    let (width, height) = match parse_dimensions(request.size.as_deref()) {
        Ok(dims) => dims,
        Err(e) => return bad_request(e),
    };
    let count = request.n.unwrap_or(1).clamp(1, 10);
    let is_b64 = request.response_format.as_deref() != Some("url");
    let socket = resolve_runtime_socket();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut data = Vec::with_capacity(count);
    for i in 0..count {
        let seed = now.wrapping_mul(1000).wrapping_add(i as u64);
        match call_generate_visual(
            &socket,
            prompt,
            request.model.as_deref(),
            width,
            height,
            seed,
        )
        .await
        {
            Ok(params) => {
                let img_path = params
                    .get("image_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if is_b64 {
                    match tokio::fs::read(img_path).await {
                        Ok(b) => data.push(ImageObject {
                            b64_json: Some(base64_encode(&b)),
                            url: None,
                            revised_prompt: Some(prompt.to_string()),
                        }),
                        Err(e) => {
                            error!("Read image failed: {e}");
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(json!({"error": {"message": e.to_string()}})),
                            )
                                .into_response();
                        }
                    }
                } else {
                    data.push(ImageObject {
                        b64_json: None,
                        url: Some(format!("file://{img_path}")),
                        revised_prompt: Some(prompt.to_string()),
                    });
                }
            }
            Err(e) => {
                error!("GenerateVisual failed: {e}");
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": {"message": e, "type": "runtime_gateway_error"}})),
                )
                    .into_response();
            }
        }
    }

    debug!("Generated {} images for '{prompt}'", data.len());
    (
        StatusCode::OK,
        Json(ImageGenerationResponse { created: now, data }),
    )
        .into_response()
}

pub async fn image_edits_handler(
    State(engine): State<Arc<RouterEngine>>,
    Json(request): Json<ImageEditRequest>,
) -> Response {
    let img_raw = request.image.as_deref().unwrap_or("").trim();
    if img_raw.is_empty() {
        return bad_request("image is required");
    }
    let is_file = img_raw.starts_with("file://") || Path::new(img_raw).exists();
    if !is_file && base64_decode(img_raw).is_none() {
        return bad_request("invalid base64 image data");
    }
    let prompt = request.prompt.as_deref().unwrap_or("").trim();
    if prompt.is_empty() {
        return bad_request("prompt is required");
    }

    image_generations_handler(
        State(engine),
        Json(ImageGenerationRequest {
            prompt: prompt.to_string(),
            model: request.model,
            n: request.n,
            size: request.size,
            quality: None,
            response_format: request.response_format,
        }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_images_reexports_codec() {
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(parse_dimensions(None), Ok((512, 512)));
    }
}
