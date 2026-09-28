//! Daemon Varlink server proof: bind a real Router1 listener, speak raw
//! Varlink frames at it, and check every method's reply shape.

use routerd_core::{RouterConfig, RouterEngine};
use routerd_daemon::varlink::bind_or_create_listener;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn socket_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("router1-test-{}-{}.sock", std::process::id(), tag))
}

/// Bind + spawn the real listener; the task dies with the test runtime.
async fn start_daemon(tag: &str) -> PathBuf {
    let path = socket_path(tag);
    let listener = bind_or_create_listener(&path).unwrap();
    let engine = Arc::new(RouterEngine::new(RouterConfig::default()));
    tokio::spawn(routerd_daemon::varlink::run_varlink_listener(listener, engine));
    // Give the accept loop a moment to start.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    path
}

async fn call(path: &PathBuf, method: &str, params: Value) -> Value {
    let mut stream = tokio::net::UnixStream::connect(path).await.unwrap();
    let mut req = serde_json::to_vec(&json!({"method": method, "parameters": params})).unwrap();
    req.push(0);
    stream.write_all(&req).await.unwrap();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await.unwrap();
        if byte[0] == 0 {
            break;
        }
        buf.push(byte[0]);
    }
    serde_json::from_slice(&buf).unwrap()
}

#[tokio::test]
async fn service_answers_info_and_descriptions() {
    let path = start_daemon("svc").await;
    let info = call(&path, "org.varlink.service.GetInfo", json!({})).await;
    assert_eq!(info["parameters"]["product"], "routerd");
    let desc = call(
        &path,
        "org.varlink.service.GetInterfaceDescription",
        json!({"interface": "io.syntrop.Router1"}),
    )
    .await;
    assert!(desc["parameters"]["description"].as_str().unwrap().contains("method GetStatus"));
    let missing = call(
        &path,
        "org.varlink.service.GetInterfaceDescription",
        json!({"interface": "nope.Nope"}),
    )
    .await;
    assert_eq!(missing["error"], "org.varlink.service.InterfaceNotFound");
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn router1_reports_status_and_lists() {
    let path = start_daemon("read").await;
    let status = call(&path, "io.syntrop.Router1.GetStatus", json!({})).await;
    assert_eq!(status["parameters"]["status"], "active");
    assert_eq!(status["parameters"]["providers_count"], 0);
    let providers = call(&path, "io.syntrop.Router1.ListProviders", json!({})).await;
    assert_eq!(providers["parameters"]["providers"], json!([]));
    let models = call(&path, "io.syntrop.Router1.ListModels", json!({})).await;
    let ids: Vec<&str> = models["parameters"]["models"].as_array().unwrap().iter().filter_map(|m| m.as_str()).collect();
    assert!(ids.contains(&"router:fast"));
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn router1_routes_tests_and_rejects_unknown() {
    let path = start_daemon("write").await;
    let route = call(
        &path,
        "io.syntrop.Router1.RouteRequest",
        json!({"model": "router:fast", "require_stream": false}),
    )
    .await;
    assert_eq!(route["parameters"]["candidates"], json!([]));
    let no_param = call(&path, "io.syntrop.Router1.TestProvider", json!({})).await;
    assert_eq!(no_param["error"], "org.varlink.service.InvalidParameter");
    let unknown = call(
        &path,
        "io.syntrop.Router1.TestProvider",
        json!({"provider_id": "nope"}),
    )
    .await;
    assert_eq!(unknown["error"], "io.syntrop.Router1.TestFailed");
    let bogus = call(&path, "io.syntrop.Router1.Nope", json!({})).await;
    assert_eq!(bogus["error"], "org.varlink.service.MethodNotFound");
    let _ = std::fs::remove_file(&path);
}
