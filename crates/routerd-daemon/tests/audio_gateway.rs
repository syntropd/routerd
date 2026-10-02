//! Integration tests verifying OpenAI audio transcription and speech gateway requests.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use routerd_core::config::RouterConfig;
use routerd_core::RouterEngine;
use routerd_daemon::gateway::create_gateway_router;
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tower::ServiceExt;

async fn run_mock_runtime_audio_server(listener: UnixListener) {
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                let req_val: Value = loop {
                    let n = stream.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.iter().position(|&b| b == 0) {
                        break serde_json::from_slice(&buf[..pos]).unwrap_or(Value::Null);
                    }
                };

                let method = req_val.get("method").and_then(|m| m.as_str()).unwrap_or("");
                if method == "io.syntrop.Runtime1.TranscribeAudio" {
                    let reply = json!({
                        "parameters": {
                            "text": "syntrop transcribed speech stream",
                            "language": "en",
                            "duration_ms": 1000
                        }
                    });
                    let mut bytes = serde_json::to_vec(&reply).unwrap_or_default();
                    bytes.push(0);
                    let _ = stream.write_all(&bytes).await;
                } else if method == "io.syntrop.Runtime1.StreamAudioOut" {
                    let dummy_pcm = vec![0u8; 320];
                    let b64 = "AAAA".repeat(80);
                    let reply = json!({
                        "parameters": {
                            "bytes_streamed": dummy_pcm.len(),
                            "sample_rate": 24000,
                            "channels": 1,
                            "pcm_base64": b64
                        }
                    });
                    let mut bytes = serde_json::to_vec(&reply).unwrap_or_default();
                    bytes.push(0);
                    let _ = stream.write_all(&bytes).await;
                }
            });
        }
    });
}

#[tokio::test]
async fn test_audio_transcriptions_and_speech_flow() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("mock_audio_runtime.sock");

    let listener = UnixListener::bind(&sock_path).unwrap();
    run_mock_runtime_audio_server(listener).await;

    std::env::set_var("SYNTROP_RUNTIME_SOCKET", sock_path.to_str().unwrap());

    let config = RouterConfig::default();
    let engine = Arc::new(RouterEngine::new(config));
    let app = create_gateway_router(engine);

    // Test 1: JSON transcription request
    let req = Request::builder()
        .method("POST")
        .uri("/v1/audio/transcriptions")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "file": "AAAA",
                "model": "whisper-1",
                "language": "en"
            })
            .to_string(),
        ))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(val["text"], "syntrop transcribed speech stream");

    // Test 2: Text response format
    let req_txt = Request::builder()
        .method("POST")
        .uri("/v1/audio/transcriptions")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "file": "AAAA",
                "response_format": "text"
            })
            .to_string(),
        ))
        .unwrap();

    let res_txt = app.clone().oneshot(req_txt).await.unwrap();
    assert_eq!(res_txt.status(), StatusCode::OK);
    let body_txt = res_txt.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(std::str::from_utf8(&body_txt).unwrap(), "syntrop transcribed speech stream");

    // Test 3: Speech request raw PCM
    let req_pcm = Request::builder()
        .method("POST")
        .uri("/v1/audio/speech")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "model": "tts-1",
                "input": "Syntrop host audio online.",
                "voice": "alloy",
                "response_format": "pcm"
            })
            .to_string(),
        ))
        .unwrap();

    let res_pcm = app.clone().oneshot(req_pcm).await.unwrap();
    assert_eq!(res_pcm.status(), StatusCode::OK);
    assert_eq!(res_pcm.headers()["content-type"], "audio/pcm");
    let body_pcm = res_pcm.into_body().collect().await.unwrap().to_bytes();
    assert!(!body_pcm.is_empty());

    // Test 4: Speech request WAV format
    let req_wav = Request::builder()
        .method("POST")
        .uri("/v1/audio/speech")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "model": "tts-1",
                "input": "Syntrop host audio online.",
                "response_format": "wav"
            })
            .to_string(),
        ))
        .unwrap();

    let res_wav = app.clone().oneshot(req_wav).await.unwrap();
    assert_eq!(res_wav.status(), StatusCode::OK);
    assert_eq!(res_wav.headers()["content-type"], "audio/wav");
    let body_wav = res_wav.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body_wav[0..4], b"RIFF");
    assert_eq!(&body_wav[8..12], b"WAVE");

    // Test 5: Validation error on empty speech
    let req_bad = Request::builder()
        .method("POST")
        .uri("/v1/audio/speech")
        .header("content-type", "application/json")
        .body(Body::from(json!({ "input": "   " }).to_string()))
        .unwrap();

    let res_bad = app.clone().oneshot(req_bad).await.unwrap();
    assert_eq!(res_bad.status(), StatusCode::BAD_REQUEST);
}
