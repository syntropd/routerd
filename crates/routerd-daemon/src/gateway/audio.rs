use super::codec::audio::{encode_wav_header, extract_multipart_field, parse_multipart_boundary};
use super::codec::images::{base64_decode, base64_encode};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
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
pub struct AudioTranscriptionRequest {
    pub file: Option<String>,
    pub audio: Option<String>,
    pub model: Option<String>,
    pub language: Option<String>,
    pub response_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioTranscriptionResponse {
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AudioSpeechRequest {
    pub model: Option<String>,
    pub input: String,
    pub voice: Option<String>,
    pub response_format: Option<String>,
    pub speed: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AudioGenerationRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub duration: Option<u32>,
    pub duration_sec: Option<u32>,
    pub bpm: Option<u32>,
    pub response_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioObject {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub duration: f32,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioGenerationResponse {
    pub created: u64,
    pub data: Vec<AudioObject>,
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

pub async fn audio_transcriptions_handler(
    State(_engine): State<Arc<RouterEngine>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ct = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    let (pcm_base64, language, resp_fmt) = if ct.contains("application/json") {
        let req: AudioTranscriptionRequest = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => return bad_request(&format!("invalid json body: {e}")),
        };
        let b64 = match req.file.or(req.audio) {
            Some(s) if !s.trim().is_empty() => s,
            _ => return bad_request("file or audio is required"),
        };
        (b64, req.language, req.response_format)
    } else if ct.contains("multipart/form-data") {
        let boundary = parse_multipart_boundary(ct);
        let raw = &body[..];
        let file_bytes = match extract_multipart_field(raw, "file", boundary) {
            Some(b) if !b.is_empty() => b,
            _ => return bad_request("file field missing in multipart payload"),
        };
        let b64 = base64_encode(file_bytes);
        let lang = extract_multipart_field(raw, "language", boundary).and_then(|b| std::str::from_utf8(b).ok().map(|s| s.trim().to_string()));
        let fmt = extract_multipart_field(raw, "response_format", boundary).and_then(|b| std::str::from_utf8(b).ok().map(|s| s.trim().to_string()));
        (b64, lang, fmt)
    } else {
        if body.is_empty() { return bad_request("request body cannot be empty"); }
        (base64_encode(&body), None, None)
    };

    let socket = resolve_runtime_socket();
    let mut params = json!({ "pcm_base64": pcm_base64 });
    if let Some(ref l) = language { params["language"] = json!(l); }

    match call_varlink(&socket, "io.syntrop.Runtime1.TranscribeAudio", params).await {
        Ok(res) => {
            let text = res.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let lang = res.get("language").and_then(|v| v.as_str()).unwrap_or("en");
            let dur = res.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
            match resp_fmt.as_deref() {
                Some("text") => (StatusCode::OK, [("content-type", "text/plain")], text.to_string()).into_response(),
                Some("verbose_json") => (StatusCode::OK, Json(json!({"task": "transcribe", "language": lang, "duration": (dur as f64) / 1000.0, "text": text}))).into_response(),
                _ => (StatusCode::OK, Json(AudioTranscriptionResponse { text: text.to_string() })).into_response(),
            }
        }
        Err(e) => {
            error!("TranscribeAudio failed: {e}");
            (StatusCode::BAD_GATEWAY, Json(json!({"error": {"message": e, "type": "runtime_gateway_error"}}))).into_response()
        }
    }
}

pub async fn audio_speech_handler(
    State(_engine): State<Arc<RouterEngine>>,
    Json(request): Json<AudioSpeechRequest>,
) -> Response {
    let input = request.input.trim();
    if input.is_empty() { return bad_request("input text is required"); }
    let socket = resolve_runtime_socket();
    let voice = request.voice.unwrap_or_else(|| "af_bella".to_string());
    let params = json!({ "text": input, "voice": voice, "sink_type": "buffer" });

    match call_varlink(&socket, "io.syntrop.Runtime1.StreamAudioOut", params).await {
        Ok(res) => {
            let pcm_b64 = res.get("pcm_base64").and_then(|v| v.as_str()).unwrap_or("");
            let pcm_bytes = base64_decode(pcm_b64).unwrap_or_default();
            if request.response_format.as_deref() == Some("wav") {
                let wav = encode_wav_header(24000, 1, &pcm_bytes);
                (StatusCode::OK, [(CONTENT_TYPE, "audio/wav")], wav).into_response()
            } else {
                (StatusCode::OK, [(CONTENT_TYPE, "audio/pcm")], pcm_bytes).into_response()
            }
        }
        Err(e) => {
            error!("StreamAudioOut failed: {e}");
            (StatusCode::BAD_GATEWAY, Json(json!({"error": {"message": e, "type": "runtime_gateway_error"}}))).into_response()
        }
    }
}

pub async fn audio_generations_handler(
    State(_engine): State<Arc<RouterEngine>>,
    Json(request): Json<AudioGenerationRequest>,
) -> Response {
    let prompt = request.prompt.trim();
    if prompt.is_empty() { return bad_request("prompt is required"); }
    let dur_sec = request.duration.or(request.duration_sec).unwrap_or(5).clamp(1, 120);
    let socket = resolve_runtime_socket();
    let mut params = json!({ "prompt": prompt, "duration_sec": dur_sec });
    if let Some(bpm) = request.bpm { params["bpm"] = json!(bpm); }
    if let Some(ref m) = request.model { params["model"] = json!(m); }

    match call_varlink(&socket, "io.syntrop.Runtime1.GenerateAudio", params).await {
        Ok(res) => {
            let path = res.get("audio_path").and_then(|v| v.as_str()).unwrap_or("");
            let sample_rate = res.get("sample_rate").and_then(|v| v.as_u64()).unwrap_or(32000) as u32;
            let duration_ms = res.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or((dur_sec as u64) * 1000);
            let duration_sec = (duration_ms as f32) / 1000.0;
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            let is_b64 = request.response_format.as_deref() == Some("b64_json");

            let obj = if is_b64 {
                match tokio::fs::read(path).await {
                    Ok(bytes) => AudioObject { b64_json: Some(base64_encode(&bytes)), url: None, duration: duration_sec, sample_rate },
                    Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"message": e.to_string()}}))).into_response(),
                }
            } else {
                AudioObject { b64_json: None, url: Some(format!("file://{path}")), duration: duration_sec, sample_rate }
            };
            (StatusCode::OK, Json(AudioGenerationResponse { created: now, data: vec![obj] })).into_response()
        }
        Err(e) => {
            error!("GenerateAudio failed: {e}");
            (StatusCode::BAD_GATEWAY, Json(json!({"error": {"message": e, "type": "runtime_gateway_error"}}))).into_response()
        }
    }
}
