//! RuntimedAdapter proof: a scripted fake Runtime1 socket answers
//! Generate/GetLoad/ListLoadedModels, and the adapter turns them into
//! OpenAI-shaped completions, health, and model lists.

use futures::StreamExt;
use routerd_core::adapters::create_adapter;
use routerd_core::config::ProviderConfig;
use routerd_core::models::{ChatCompletionRequest, ChatMessage};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

fn cfg(socket: &PathBuf) -> ProviderConfig {
    serde_json::from_value(json!({
        "id": "runtimed-local",
        "kind": "runtimed",
        "base_url": socket.to_string_lossy(),
        "timeout_ms": 5000,
        "models": ["gemma-4-E2B-it-Q4_K_M"],
    }))
    .unwrap()
}

fn request() -> ChatCompletionRequest {
    ChatCompletionRequest {
        model: "gemma-4-E2B-it-Q4_K_M".to_string(),
        messages: vec![ChatMessage::new("user", "Say hello.")],
        temperature: None,
        top_p: None,
        max_tokens: None,
        max_completion_tokens: None,
        stream: None,
        tier: None,
        reasoning_budget: None,
        max_thinking_tokens: None,
        reasoning_content: None,
        reasoning_effort: None,
        extra: HashMap::new(),
    }
}

/// Serve exactly one connection, then return what the client asked for.
async fn serve_once(listener: UnixListener) -> Value {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let asked = loop {
        let n = stream.read(&mut chunk).await.unwrap();
        assert!(n > 0, "client hung up");
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.iter().position(|&b| b == 0) {
            break serde_json::from_slice::<Value>(&buf[..pos]).unwrap();
        }
    };
    let method = asked.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let reply = match method {
        "io.syntrop.Runtime1.Generate" => json!({
            "parameters": {
                "result": {
                    "text": "Hello there.",
                    "prompt_tokens": 5,
                    "completion_tokens": 3,
                    "finish_reason": "stop",
                    "duration_ms": 12
                }
            }
        }),
        "io.syntrop.Runtime1.GetLoad" => json!({
            "parameters": { "available_slots": 4, "max_slots": 4, "used_bytes": 0, "models": [] }
        }),
        "io.syntrop.Runtime1.ListLoadedModels" => json!({
            "parameters": { "models": [{ "name": "gemma-4-E2B-it-Q4_K_M" }] }
        }),
        other => json!({ "error": format!("unknown method {other}") }),
    };
    let mut bytes = serde_json::to_vec(&reply).unwrap();
    bytes.push(0);
    stream.write_all(&bytes).await.unwrap();
    asked
}

fn socket_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("runtimed-adapter-test-{}-{}.sock", std::process::id(), tag))
}

#[tokio::test]
async fn generate_becomes_chat_completion() {
    let path = socket_path("gen");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let server = tokio::spawn(async move { serve_once(listener).await });
    let adapter = create_adapter(&cfg(&path));
    let response = adapter
        .chat_completion("gemma-4-E2B-it-Q4_K_M", &request())
        .await
        .unwrap();
    let asked = server.await.unwrap();
    // The adapter must speak the real Runtime1.Generate contract.
    assert_eq!(
        asked.get("method").and_then(|m| m.as_str()),
        Some("io.syntrop.Runtime1.Generate")
    );
    let params = asked.get("parameters").unwrap();
    assert_eq!(
        params.get("model").and_then(|m| m.as_str()),
        Some("gemma-4-E2B-it-Q4_K_M")
    );
    assert!(params.get("prompt").and_then(|p| p.as_str()).unwrap().contains("Say hello."));
    assert_eq!(params.get("max_tokens").and_then(|m| m.as_u64()), Some(256));
    // ...and translate the GenerationResult faithfully.
    assert_eq!(response.model, "gemma-4-E2B-it-Q4_K_M");
    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].message.content_as_str(), "Hello there.");
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));
    let usage = response.usage.unwrap();
    assert_eq!((usage.prompt_tokens, usage.completion_tokens, usage.total_tokens), (5, 3, 8));
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn health_and_models_use_runtimed_methods() {
    let path = socket_path("health");
    let _ = std::fs::remove_file(&path);
    let adapter = create_adapter(&cfg(&path));
    // No socket at all: unhealthy, configured list as fallback.
    assert!(!adapter.health_check().await.unwrap());
    assert_eq!(adapter.list_models().await.unwrap(), vec!["gemma-4-E2B-it-Q4_K_M".to_string()]);

    let listener = UnixListener::bind(&path).unwrap();
    let server = tokio::spawn(async move { serve_once(listener).await });
    assert!(adapter.health_check().await.unwrap());
    let asked = server.await.unwrap();
    assert_eq!(
        asked.get("method").and_then(|m| m.as_str()),
        Some("io.syntrop.Runtime1.GetLoad")
    );

    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let server = tokio::spawn(async move { serve_once(listener).await });
    assert_eq!(adapter.list_models().await.unwrap(), vec!["gemma-4-E2B-it-Q4_K_M".to_string()]);
    let asked = server.await.unwrap();
    assert_eq!(
        asked.get("method").and_then(|m| m.as_str()),
        Some("io.syntrop.Runtime1.ListLoadedModels")
    );
    let _ = std::fs::remove_file(&path);
}

/// Serve once, but wait `delay` before answering (cold model load).
async fn serve_once_slow(listener: UnixListener, delay: std::time::Duration) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        assert!(n > 0, "client hung up");
        buf.extend_from_slice(&chunk[..n]);
        if buf.iter().position(|&b| b == 0).is_some() {
            break;
        }
    }
    tokio::time::sleep(delay).await;
    let reply = json!({
        "parameters": {
            "result": {
                "text": "Slow hello.",
                "prompt_tokens": 5,
                "completion_tokens": 3,
                "finish_reason": "stop",
                "duration_ms": 1000
            }
        }
    });
    let mut bytes = serde_json::to_vec(&reply).unwrap();
    bytes.push(0);
    stream.write_all(&bytes).await.unwrap();
}

#[tokio::test]
async fn slow_generate_survives_snappy_configured_timeout() {
    // Regression: a 30s configured timeout killed cold 3GB loads with
    // "runtimed read timed out". Completions get a generous floor.
    let path = socket_path("slow");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let server = tokio::spawn(async move {
        serve_once_slow(listener, std::time::Duration::from_secs(3)).await;
    });
    let mut config = cfg(&path);
    config.timeout_ms = 300;
    let adapter = create_adapter(&config);
    let response = adapter
        .chat_completion("gemma-4-E2B-it-Q4_K_M", &request())
        .await
        .expect("slow generate must survive the floor");
    server.await.unwrap();
    assert_eq!(response.choices[0].message.content_as_str(), "Slow hello.");
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn stream_emits_single_chunk_then_done() {
    let path = socket_path("stream");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let server = tokio::spawn(async move { serve_once(listener).await });
    let adapter = create_adapter(&cfg(&path));
    let mut stream = adapter
        .chat_completion_stream("gemma-4-E2B-it-Q4_K_M", &request())
        .await
        .unwrap();
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        text.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
    }
    server.await.unwrap();
    assert!(text.contains("Hello there."), "missing chunk: {text}");
    assert!(text.contains("data: [DONE]"), "missing terminator: {text}");
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn length_truncation_in_reasoning_falls_back() {
    let path = socket_path("trunc");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let server = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut b = [0u8; 1024]; let _ = s.read(&mut b).await.unwrap();
        let rep = json!({"parameters": {"result": {"text": "<think>partial thought", "prompt_tokens": 2, "completion_tokens": 3, "finish_reason": "length", "duration_ms": 10}}});
        let mut v = serde_json::to_vec(&rep).unwrap(); v.push(0); s.write_all(&v).await.unwrap();
    });
    let adapter = create_adapter(&cfg(&path));
    let res = adapter.chat_completion("gemma-4-E2B-it-Q4_K_M", &request()).await.unwrap();
    server.await.unwrap();
    assert_eq!(res.choices[0].message.content_as_str(), "partial thought");
    assert_eq!(res.choices[0].message.reasoning_content.as_deref(), Some("partial thought"));
    let _ = std::fs::remove_file(&path);
}
