//! OpenAI-compatible /v1/audio HTTP gateway endpoints (transcriptions & speech).

use super::images_codec::{base64_decode, base64_encode};
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
    let mut stream = UnixStream::connect(sock)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
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

pub async fn audio_transcriptions_handler(
    State(_engine): State<Arc<RouterEngine>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ct = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

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
        // Parse multipart body boundaries
        let raw = &body[..];
        let file_bytes = match extract_multipart_field(raw, "file") {
            Some(b) if !b.is_empty() => b,
            _ => return bad_request("file field missing in multipart payload"),
        };
        let b64 = base64_encode(file_bytes);
        let lang = extract_multipart_field(raw, "language").and_then(|b| std::str::from_utf8(b).ok().map(String::from));
        let fmt = extract_multipart_field(raw, "response_format").and_then(|b| std::str::from_utf8(b).ok().map(String::from));
        (b64, lang, fmt)
    } else {
        if body.is_empty() {
            return bad_request("request body cannot be empty");
        }
        let b64 = base64_encode(&body);
        (b64, None, None)
    };

    let socket = resolve_runtime_socket();
    let mut params = json!({ "pcm_base64": pcm_base64 });
    if let Some(ref l) = language {
        params["language"] = json!(l);
    }

    match call_varlink(&socket, "io.syntrop.Runtime1.TranscribeAudio", params).await {
        Ok(res) => {
            let text = res.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let lang = res.get("language").and_then(|v| v.as_str()).unwrap_or("en");
            let dur = res.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);

            match resp_fmt.as_deref() {
                Some("text") => (StatusCode::OK, [("content-type", "text/plain")], text.to_string()).into_response(),
                Some("verbose_json") => (
                    StatusCode::OK,
                    Json(json!({
                        "task": "transcribe",
                        "language": lang,
                        "duration": (dur as f64) / 1000.0,
                        "text": text,
                    })),
                )
                    .into_response(),
                _ => (StatusCode::OK, Json(AudioTranscriptionResponse { text: text.to_string() })).into_response(),
            }
        }
        Err(e) => {
            error!("TranscribeAudio failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {"message": e, "type": "runtime_gateway_error"}})),
            )
                .into_response()
        }
    }
}

pub async fn audio_speech_handler(
    State(_engine): State<Arc<RouterEngine>>,
    Json(request): Json<AudioSpeechRequest>,
) -> Response {
    let input = request.input.trim();
    if input.is_empty() {
        return bad_request("input text is required");
    }

    let socket = resolve_runtime_socket();
    let voice = request.voice.unwrap_or_else(|| "af_bella".to_string());
    let params = json!({
        "text": input,
        "voice": voice,
        "sink_type": "buffer"
    });

    match call_varlink(&socket, "io.syntrop.Runtime1.StreamAudioOut", params).await {
        Ok(res) => {
            let pcm_b64 = res.get("pcm_base64").and_then(|v| v.as_str()).unwrap_or("");
            let pcm_bytes = base64_decode(pcm_b64).unwrap_or_default();
            let is_wav = request.response_format.as_deref() == Some("wav");

            if is_wav {
                let wav = encode_wav_header(24000, 1, &pcm_bytes);
                (StatusCode::OK, [(CONTENT_TYPE, "audio/wav")], wav).into_response()
            } else {
                (StatusCode::OK, [(CONTENT_TYPE, "audio/pcm")], pcm_bytes).into_response()
            }
        }
        Err(e) => {
            error!("StreamAudioOut failed: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {"message": e, "type": "runtime_gateway_error"}})),
            )
                .into_response()
        }
    }
}

fn extract_multipart_field<'a>(data: &'a [u8], name: &str) -> Option<&'a [u8]> {
    let needle = format!("name=\"{name}\"");
    let pos = data.windows(needle.len()).position(|w| w == needle.as_bytes())?;
    let after_header = &data[pos + needle.len()..];
    let delim = b"\r\n\r\n";
    let body_start = after_header.windows(4).position(|w| w == delim)? + 4;
    let payload = &after_header[body_start..];
    let end = payload.windows(2).position(|w| w == b"\r\n").unwrap_or(payload.len());
    Some(&payload[..end])
}

fn encode_wav_header(sample_rate: u32, channels: u16, pcm: &[u8]) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let riff_size = 36 + data_len;
    let byte_rate = sample_rate * channels as u32 * 2;
    let block_align = channels * 2;

    let mut buf = Vec::with_capacity(44 + pcm.len());
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&riff_size.to_le_bytes());
    buf.extend_from_slice(b"WAVEfmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    buf.extend_from_slice(pcm);
    buf
}
