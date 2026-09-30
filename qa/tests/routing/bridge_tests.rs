//! VarlinkBridgeAdapter proof: a scripted fake Inference1 socket answers
//! StreamInference/ListModels, and the adapter turns them into OpenAI-shaped
//! completions, SSE streams, health, and model lists.

use futures::StreamExt;
use routerd_core::adapters::create_adapter;
use routerd_core::config::ProviderConfig;
use routerd_core::models::{ChatCompletionRequest, ChatMessage, ProviderModelConfig};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

fn cfg(socket: &Path) -> ProviderConfig {
    ProviderConfig {
        id: "syntrop-local".to_string(),
        name: "Syntrop Local".to_string(),
        kind: "varlink".to_string(),
        base_url: socket.to_string_lossy().to_string(),
        api_key: None,
        tier: "fast".to_string(),
        weight: 1.3,
        enabled: true,
        timeout_ms: 5000,
        models: vec![ProviderModelConfig {
            name: "local-qwen".to_string(),
            max_context_tokens: 16384,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            avg_latency_ms: 40.0,
            tokens_per_second: 300.0,
            tier: Some("fast".to_string()),
        }],
    }
}

fn request() -> ChatCompletionRequest {
    ChatCompletionRequest {
        model: "local-qwen".to_string(),
        messages: vec![ChatMessage::new("user", "Say hello.")],
        max_tokens: Some(64),
        ..Default::default()
    }
}

async fn read_frame(stream: &mut tokio::net::UnixStream) -> Value {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        assert!(n > 0, "client hung up");
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.iter().position(|&b| b == 0) {
            return serde_json::from_slice(&buf[..pos]).unwrap();
        }
    }
}

async fn write_frame(stream: &mut tokio::net::UnixStream, val: &Value) {
    let mut bytes = serde_json::to_vec(val).unwrap();
    bytes.push(0);
    // Health probes close without reading; a failed write is not an error here.
    let _ = stream.write_all(&bytes).await;
}

/// Fake Inference1: every connection gets one request, then the scripted reply frames.
async fn serve(listener: UnixListener) {
    loop {
        let (mut stream, _) = listener.accept().await.unwrap();
        tokio::spawn(async move {
            let asked = read_frame(&mut stream).await;
            let method = asked.get("method").and_then(|m| m.as_str()).unwrap_or("");
            match method {
                "io.syntrop.Inference1.StreamInference" => {
                    write_frame(
                        &mut stream,
                        &json!({"parameters": {"chunk": "Hello "}, "continues": true}),
                    )
                    .await;
                    write_frame(
                        &mut stream,
                        &json!({"parameters": {"chunk": "there."}, "continues": false}),
                    )
                    .await;
                }
                "io.syntrop.Inference1.ListModels" => {
                    write_frame(
                        &mut stream,
                        &json!({"parameters": {"models": [{"id": "live-a"}, "live-b"]}}),
                    )
                    .await;
                }
                _ => {
                    write_frame(&mut stream, &json!({"parameters": {}})).await;
                }
            }
        });
    }
}

fn socket_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("bridge-test-{}-{}.sock", std::process::id(), tag))
}

async fn start_server(tag: &str) -> PathBuf {
    let path = socket_path(tag);
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    tokio::spawn(serve(listener));
    path
}

#[tokio::test]
async fn bridge_accumulates_stream_into_completion() {
    let path = start_server("chat").await;
    let adapter = create_adapter(&cfg(&path));
    let response = adapter.chat_completion("local-qwen", &request()).await.unwrap();
    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].message.content_as_str(), "Hello there.");
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));
    let usage = response.usage.unwrap();
    assert_eq!(usage.total_tokens, usage.prompt_tokens + usage.completion_tokens);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn bridge_streams_sse_chunks_then_done() {
    let path = start_server("stream").await;
    let adapter = create_adapter(&cfg(&path));
    let mut stream = adapter.chat_completion_stream("local-qwen", &request()).await.unwrap();
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        text.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
    }
    assert!(text.contains("Hello "), "missing first chunk: {text}");
    assert!(text.contains("there."), "missing second chunk: {text}");
    assert!(text.contains("data: [DONE]"), "missing terminator: {text}");
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn bridge_health_and_models_prefer_live_socket() {
    let path = socket_path("probe");
    let _ = std::fs::remove_file(&path);
    let adapter = create_adapter(&cfg(&path));
    assert!(!adapter.health_check().await.unwrap());
    assert_eq!(adapter.list_models().await.unwrap(), vec!["local-qwen".to_string()]);

    let listener = UnixListener::bind(&path).unwrap();
    tokio::spawn(serve(listener));
    assert!(adapter.health_check().await.unwrap());
    assert_eq!(adapter.list_models().await.unwrap(), vec!["live-a".to_string(), "live-b".to_string()]);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn bridge_slow_stream_survives_snappy_configured_timeout() {
    let path = socket_path("slow-stream");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await;
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        write_frame(
            &mut stream,
            &json!({"parameters": {"chunk": "Slow "}, "continues": true}),
        )
        .await;
        write_frame(
            &mut stream,
            &json!({"parameters": {"chunk": "stream."}, "continues": false}),
        )
        .await;
    });

    let mut config = cfg(&path);
    config.timeout_ms = 300;
    let adapter = create_adapter(&config);
    let mut stream = adapter
        .chat_completion_stream("local-qwen", &request())
        .await
        .expect("slow stream must survive the generate floor");
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        text.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
    }
    assert!(text.contains("Slow "), "missing first chunk: {text}");
    assert!(text.contains("stream."), "missing second chunk: {text}");
    assert!(text.contains("data: [DONE]"), "missing terminator: {text}");
    let _ = std::fs::remove_file(&path);
}

