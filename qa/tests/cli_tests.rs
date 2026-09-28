//! End-to-end tests for `routerctl` verbs (status/providers/models/route/
//! info/default/ask/test). A scripted Varlink + HTTP mock pair stands in
//! for the daemon; the real binary is driven with `--socket`/`--http-url`.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;

fn routerctl_bin() -> PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("routerctl")
}

fn run(args: &[&str]) -> (bool, String) {
    let out = Command::new(routerctl_bin()).args(args).output().unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

fn varlink_params(method: &str) -> Value {
    match method {
        "io.syntrop.Router1.GetStatus" => json!({
            "status": "active", "version": "0.3.9", "uptime_seconds": 7,
            "total_requests": 10, "active_requests": 0, "providers_count": 1,
            "healthy_providers_count": 1, "psi_level": "Normal",
            "psi_memory_some": 1.0, "rss_bytes": 1000, "rss_mb": 1.0
        }),
        "io.syntrop.Router1.ListProviders" => json!({"providers": [{
            "id": "p1", "kind": "openai", "tier": "fast", "weight": 1.0,
            "is_healthy": true, "last_latency_ms": 3.0,
            "total_requests": 9, "models": ["m1"]
        }]}),
        "io.syntrop.Router1.ListModels" => json!({"models": ["m1"]}),
        "io.syntrop.Router1.RouteRequest" => json!({"candidates": [{
            "provider_id": "p1", "model_name": "m1", "total_score": 90.0,
            "speed_score": 90.0, "cost_score": 90.0, "capability_score": 90.0,
            "reason": "ok"
        }]}),
        "io.syntrop.Router1.TestProvider" => json!({
            "provider_id": "p1", "healthy": true, "latency_ms": 2.0, "error": null
        }),
        "org.varlink.service.GetInfo" => json!({
            "vendor": "syntropd", "product": "routerd", "version": "0.3.9",
            "url": "x", "interfaces": ["org.varlink.service", "io.syntrop.Router1"]
        }),
        "org.varlink.service.GetInterfaceDescription" => {
            json!({"description": "interface io.syntrop.Router1"})
        }
        _ => json!({}),
    }
}

fn start_varlink_mock(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("cli-vl-{}-{}.sock", std::process::id(), tag));
    let _ = std::fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let (mut reader, mut writer) = (conn.try_clone().unwrap(), conn);
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                if reader.read_exact(&mut byte).is_err() || byte[0] == 0 {
                    break;
                }
                buf.push(byte[0]);
            }
            let asked: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
            let m = asked.get("method").and_then(|v| v.as_str()).unwrap_or("");
            let mut reply = serde_json::to_vec(&json!({"parameters": varlink_params(m)})).unwrap();
            reply.push(0);
            let _ = writer.write_all(&reply);
        }
    });
    path
}

fn start_http_mock() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let (mut reader, mut writer) = (conn.try_clone().unwrap(), conn);
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if reader.read_exact(&mut byte).is_err() {
                    break;
                }
                head.push(byte[0]);
            }
            let head_text = String::from_utf8_lossy(&head).into_owned();
            let len = head_text
                .lines()
                .find_map(|l| l.strip_prefix("content-length: ").or_else(|| l.strip_prefix("Content-Length: ")))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            let streamed = String::from_utf8_lossy(&body).contains("\"stream\":true");
            let payload = if streamed {
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\ndata: [DONE]\n\n".to_string()
            } else {
                json!({"choices": [{"message": {"content": "4"}}]}).to_string()
            };
            let ctype = if streamed { "text/event-stream" } else { "application/json" };
            let _ = writer.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
                    payload.len()
                )
                .as_bytes(),
            );
        }
    });
    url
}

fn globals(sock: &PathBuf, http: &str) -> (String, String) {
    (sock.to_string_lossy().into_owned(), http.to_string())
}

#[test]
fn status_reports_active_json() {
    let (sock, http) = globals(&start_varlink_mock("status"), &start_http_mock());
    let (ok, out) = run(&["status", "--json", "--socket", &sock, "--http-url", &http]);
    assert!(ok, "{out}");
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["status"], "active");
}

#[test]
fn providers_and_models_json() {
    let (sock, http) = globals(&start_varlink_mock("prov"), &start_http_mock());
    let (ok, out) = run(&["providers", "list", "--json", "--socket", &sock, "--http-url", &http]);
    assert!(ok, "{out}");
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["providers"][0]["id"], "p1");
    let (ok, out) = run(&["providers", "test", "p1", "--json", "--socket", &sock, "--http-url", &http]);
    assert!(ok, "{out}");
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap()[0]["healthy"], true);
    let (ok, out) = run(&["models", "--json", "--socket", &sock, "--http-url", &http]);
    assert!(ok, "{out}");
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), json!(["m1"]));
}

#[test]
fn route_simulates_json() {
    let (sock, http) = globals(&start_varlink_mock("route"), &start_http_mock());
    let (ok, out) = run(&["route", "m1", "--json", "--socket", &sock, "--http-url", &http]);
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["candidates"][0]["model_name"], "m1");
}

#[test]
fn info_introspects_json() {
    let (sock, http) = globals(&start_varlink_mock("info"), &start_http_mock());
    let (ok, out) = run(&["info", "--json", "--socket", &sock, "--http-url", &http]);
    assert!(ok, "{out}");
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["vendor"], "syntropd");
    let (ok, out) = run(&["info", "io.syntrop.Router1", "--json", "--socket", &sock, "--http-url", &http]);
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert!(v["idl"].as_str().unwrap().contains("interface"));
}

#[test]
fn default_show_and_set() {
    let dir = std::env::temp_dir().join(format!("cli-def-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("routerd.toml");
    std::fs::write(&cfg, "[[providers]]\nid = \"p1\"\nenabled = true\nmodels = [\"m1\"]\n").unwrap();
    let c = cfg.to_string_lossy().into_owned();
    let (ok, out) = run(&["default", "--config", &c]);
    assert!(ok, "{out}");
    assert!(out.contains("scoring decides"), "{out}");
    let (ok, out) = run(&["default", "m1", "--config", &c, "--no-reload"]);
    assert!(ok, "{out}");
    let (ok, out) = run(&["default", "--config", &c, "--json"]);
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["fast"], "m1");
    assert_eq!(v["hard"], "m1");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ask_returns_reply_json() {
    let (sock, http) = globals(&start_varlink_mock("ask"), &start_http_mock());
    let (ok, out) = run(&["ask", "-m", "m1", "--json", "--socket", &sock, "--http-url", &http, "what is 2+2"]);
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["success"], true);
    assert_eq!(v["response"], "4");
}

#[test]
fn benchmark_measures_json() {
    let (sock, http) = globals(&start_varlink_mock("bench"), &start_http_mock());
    let (ok, out) = run(&["test", "-m", "m1", "--json", "--prompt", "hi", "--socket", &sock, "--http-url", &http]);
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["success"], true);
    assert_eq!(v["chunks"], 2);
    assert_eq!(v["response"], "Hello");
    assert!(v["ttft_ms"].as_f64().unwrap() > 0.0);
}
