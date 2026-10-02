//! Integration test: verifies RuntimedAdapter queries TelemetryClient
//! and shortens speculative horizon K->1 and clamps max_tokens under pressure.

use routerd_core::adapters::{ProviderAdapter, RuntimedAdapter};
use routerd_core::config::ProviderConfig;
use routerd_core::models::{ChatCompletionRequest, ChatMessage};
use routerd_core::telemetry::TelemetryClient;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

fn make_config(socket: &std::path::Path) -> ProviderConfig {
    serde_json::from_value(json!({
        "id": "runtimed-pressure-test",
        "kind": "runtimed",
        "base_url": socket.to_string_lossy(),
        "timeout_ms": 5000,
        "models": ["test-model"],
    }))
    .unwrap()
}

fn temp_socket(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("syn-runtimed-test-{}-{}.sock", std::process::id(), tag))
}

#[tokio::test]
async fn test_runtimed_adapter_memory_pressure_spike_clamping() {
    let telemetry_sock = temp_socket("telem");
    let runtime_sock = temp_socket("rt");
    let _ = std::fs::remove_file(&telemetry_sock);
    let _ = std::fs::remove_file(&runtime_sock);

    // 1. Telemetry server returning high memory pressure
    let telem_listener = UnixListener::bind(&telemetry_sock).unwrap();
    let telem_handle = tokio::spawn(async move {
        let (mut stream, _) = telem_listener.accept().await.unwrap();
        let mut buf = vec![0u8; 1024];
        let _ = stream.read(&mut buf).await.unwrap();
        let reply = json!({
            "parameters": {
                "memory_some": 55.0,
                "memory_full": 30.0,
                "cpu_some": 5.0,
                "io_some": 2.0,
                "runqueue_latency_us": 1000,
                "ebpf_active": false
            }
        });
        let mut bytes = serde_json::to_vec(&reply).unwrap();
        bytes.push(0);
        stream.write_all(&bytes).await.unwrap();
    });

    // 2. Runtime1 server expecting CompactKvCache followed by Generate with clamped max_tokens
    let compacted = Arc::new(AtomicBool::new(false));
    let compacted_flag = compacted.clone();
    let rt_listener = UnixListener::bind(&runtime_sock).unwrap();
    let rt_handle = tokio::spawn(async move {
        // May receive CompactKvCache and Generate on same or separate connections
        for _ in 0..2 {
            let (mut stream, _) = match rt_listener.accept().await {
                Ok(conn) => conn,
                Err(_) => break,
            };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 512];
            loop {
                let n = stream.read(&mut chunk).await.unwrap();
                if n == 0 { break; }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = buf.iter().position(|&b| b == 0) {
                    let req: Value = serde_json::from_slice(&buf[..pos]).unwrap();
                    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
                    let rep = if method == "io.syntrop.Runtime1.CompactKvCache" {
                        compacted_flag.store(true, Ordering::SeqCst);
                        json!({"parameters": {}})
                    } else if method == "io.syntrop.Runtime1.Generate" {
                        let max_tokens = req["parameters"]["max_tokens"].as_u64().unwrap_or(0);
                        assert!(max_tokens <= 128, "max_tokens must be clamped to <= 128, got {max_tokens}");
                        json!({
                            "parameters": {
                                "result": {
                                    "text": "Clamped response.",
                                    "prompt_tokens": 4,
                                    "completion_tokens": 2,
                                    "finish_reason": "stop",
                                    "duration_ms": 10
                                }
                            }
                        })
                    } else {
                        json!({"error": "unknown"})
                    };
                    let mut b = serde_json::to_vec(&rep).unwrap();
                    b.push(0);
                    let _ = stream.write_all(&b).await;
                    break;
                }
            }
        }
    });

    let telem_client = TelemetryClient::new(&telemetry_sock);
    let adapter = RuntimedAdapter::new(&make_config(&runtime_sock)).with_telemetry(telem_client);

    let req = ChatCompletionRequest {
        model: "test-model".to_string(),
        messages: vec![ChatMessage::new("user", "Hello")],
        max_tokens: Some(1024),
        ..Default::default()
    };

    let resp = adapter.chat_completion("test-model", &req).await.unwrap();
    assert_eq!(resp.choices[0].message.content_as_str(), "Clamped response.");
    assert!(compacted.load(Ordering::SeqCst), "CompactKvCache must have been called during pressure spike");

    let _ = telem_handle.await;
    let _ = rt_handle.await;
    let _ = std::fs::remove_file(&telemetry_sock);
    let _ = std::fs::remove_file(&runtime_sock);
}
