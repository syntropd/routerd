//! Integration tests verifying OpenAI image generation and edit gateway requests.

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

async fn run_mock_runtime_server(listener: UnixListener, img_file: String) {
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let img = img_file.clone();
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
                if method == "io.syntrop.Runtime1.GenerateVisual" {
                    let reply = json!({
                        "parameters": {
                            "image_path": img,
                            "bytes": 8,
                            "width": 512,
                            "height": 512,
                            "format": "png"
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
async fn test_images_generations_and_edits_flow() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("mock_runtime.sock");
    let img_path = dir.path().join("test_img.png");
    tokio::fs::write(&img_path, b"\x89PNG\r\n\x1a\n")
        .await
        .unwrap();

    let listener = UnixListener::bind(&sock_path).unwrap();
    run_mock_runtime_server(listener, img_path.to_string_lossy().to_string()).await;

    std::env::set_var("SYNTROP_RUNTIME_SOCKET", sock_path.to_str().unwrap());

    let config = RouterConfig::default();
    let engine = Arc::new(RouterEngine::new(config));
    let app = create_gateway_router(engine);

    // Test 1: Generations with b64_json
    let req = Request::builder()
        .method("POST")
        .uri("/v1/images/generations")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "prompt": "An art deco server rack",
                "size": "512x512",
                "response_format": "b64_json"
            })
            .to_string(),
        ))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&body).unwrap();
    assert!(val.get("created").is_some());
    let data = val.get("data").and_then(|d| d.as_array()).unwrap();
    assert_eq!(data.len(), 1);
    assert!(data[0].get("b64_json").and_then(|b| b.as_str()).is_some());

    // Test 2: Generations with URL format
    let req_url = Request::builder()
        .method("POST")
        .uri("/v1/images/generations")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "prompt": "Cybernetic mountain",
                "response_format": "url"
            })
            .to_string(),
        ))
        .unwrap();

    let res_url = app.clone().oneshot(req_url).await.unwrap();
    assert_eq!(res_url.status(), StatusCode::OK);
    let body_url = res_url.into_body().collect().await.unwrap().to_bytes();
    let val_url: Value = serde_json::from_slice(&body_url).unwrap();
    let data_url = val_url.get("data").and_then(|d| d.as_array()).unwrap();
    assert!(data_url[0]
        .get("url")
        .and_then(|u| u.as_str())
        .unwrap()
        .starts_with("file://"));

    // Test 3: Edits endpoint with valid base64 image
    let req_edit = Request::builder()
        .method("POST")
        .uri("/v1/images/edits")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "image": "Zm9v",
                "prompt": "Add subtle neon glow",
                "response_format": "b64_json"
            })
            .to_string(),
        ))
        .unwrap();

    let res_edit = app.clone().oneshot(req_edit).await.unwrap();
    assert_eq!(res_edit.status(), StatusCode::OK);

    // Test 4: Missing prompt validation
    let req_bad = Request::builder()
        .method("POST")
        .uri("/v1/images/generations")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "prompt": "   "
            })
            .to_string(),
        ))
        .unwrap();

    let res_bad = app.clone().oneshot(req_bad).await.unwrap();
    assert_eq!(res_bad.status(), StatusCode::BAD_REQUEST);

    // Test 5: Edits missing image parameter
    let req_no_img = Request::builder()
        .method("POST")
        .uri("/v1/images/edits")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "prompt": "Missing image input"
            })
            .to_string(),
        ))
        .unwrap();

    let res_no_img = app.clone().oneshot(req_no_img).await.unwrap();
    assert_eq!(res_no_img.status(), StatusCode::BAD_REQUEST);

    // Test 6: Edits with invalid base64 image
    let req_corrupt_b64 = Request::builder()
        .method("POST")
        .uri("/v1/images/edits")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "image": "not-valid-base64!",
                "prompt": "Fix corrupted image"
            })
            .to_string(),
        ))
        .unwrap();

    let res_corrupt_b64 = app.clone().oneshot(req_corrupt_b64).await.unwrap();
    assert_eq!(res_corrupt_b64.status(), StatusCode::BAD_REQUEST);

    // Test 7: Invalid size rejection
    let req_bad_size = Request::builder()
        .method("POST")
        .uri("/v1/images/generations")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "prompt": "A mountain",
                "size": "9999x9999"
            })
            .to_string(),
        ))
        .unwrap();

    let res_bad_size = app.clone().oneshot(req_bad_size).await.unwrap();
    assert_eq!(res_bad_size.status(), StatusCode::BAD_REQUEST);
}
