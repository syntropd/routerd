//! End-to-end tests for `routerctl setup` (verified-providers flow).
//!
//! These drive the real binary with piped stdin. The live-enumeration test
//! uses a local mock OpenAI server; nothing here needs the real internet.

use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn routerctl_bin() -> PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("routerctl")
}

fn temp_paths(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir =
        std::env::temp_dir().join(format!("routerctl_setup_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("routerd.toml");
    let models = dir.join("models");
    (dir, cfg, models)
}

fn run_setup(cfg: &PathBuf, models: &PathBuf, stdin_text: &str) -> (bool, String) {
    let mut child = Command::new(routerctl_bin())
        .args(["setup", "--config"])
        .arg(cfg)
        .args(["--models-dir"])
        .arg(models)
        .arg("--no-reload")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("routerctl binary must exist (cargo build first)");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin_text.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

fn run_default(cfg: &PathBuf, model: Option<&str>) -> (bool, String) {
    let mut cmd = Command::new(routerctl_bin());
    cmd.arg("default").args(["--config"]).arg(cfg).arg("--no-reload");
    if let Some(m) = model {
        cmd.arg(m);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = cmd.output().expect("routerctl binary must exist");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

/// Skip everything: providers end up disabled, config still parses.
#[test]
fn test_setup_skip_all_disables() {
    let (_dir, cfg, models) = temp_paths("skip");
    // custom no, default EOF-skips
    let (ok, text) = run_setup(&cfg, &models, "n\n");
    assert!(ok, "setup must exit 0 on skips. output:\n{}", text);
    let content = std::fs::read_to_string(&cfg).unwrap();
    let doc: toml::Value = content.parse().unwrap();
    let providers = doc
        .get("providers")
        .and_then(|p| p.as_array())
        .expect("providers array present");
    assert!(providers
        .iter()
        .any(|t| t.get("id").and_then(|v| v.as_str()) == Some("ollama-lan-1")));
    for t in providers {
        let id = t.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let enabled = t.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
        match id {
            // Live localhost Ollama may register itself; it verified by answering.
            "local-ollama" => {}
            // No model files are staged in this test, so setup never touches
            // the template's syntrop-local entry: it must stay off either way.
            "syntrop-local" => {
                assert!(!enabled, "syntrop-local must stay off with no model files");
            }
            _ => assert!(!enabled, "{} must be off, nothing verified", id),
        }
    }
    assert!(text.contains("Verified providers"));
}

/// A mock OpenAI server: enumerated models get stored and enabled.
#[test]
fn test_setup_enumerates_from_live_server() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in listener.incoming().take(1) {
            if let Ok(mut s) = stream {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let body = r#"{"data":[{"id":"mock-a"},{"id":"mock-b"}]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        }
    });
    let (_dir, cfg, models) = temp_paths("live");
    // custom yes + id/name/url/empty-key, default EOF-skips
    let input = format!("y\nmockprov\nMock\nhttp://127.0.0.1:{}\n\n", port);
    let (ok, text) = run_setup(&cfg, &models, &input);
    assert!(ok, "setup must exit 0. output:\n{}", text);
    let content = std::fs::read_to_string(&cfg).unwrap();
    assert!(
        content.contains("mock-a"),
        "enumerated ids stored:\n{}",
        content
    );
    assert!(content.contains("mock-b"));
    assert!(content.contains("id = \"mockprov\""));
    assert!(text.contains("LIVE"));
    server.join().unwrap();
}

/// Pre-existing cloud entries are switched off and their keys stripped,
/// without ever being pinged. Local entries are unaffected.
#[test]
fn test_setup_switches_off_cloud() {
    let (_dir, cfg, models) = temp_paths("cloudoff");
    std::fs::write(
        &cfg,
        r#"
[[providers]]
id = "minimax"
kind = "minimax"
base_url = "https://api.minimaxi.chat/v1"
api_key = "sk-live-key-must-go"
enabled = true
models = ["MiniMax-M3"]

[[providers]]
id = "lan-one"
kind = "ollama"
base_url = "http://127.0.0.1:9"
enabled = true
models = ["local-still-probed"]
"#,
    )
    .unwrap();
    let (ok, text) = run_setup(&cfg, &models, "n\n");
    assert!(ok, "setup must exit 0. output:\n{}", text);
    assert_eq!(
        text.matches("external (local-only mode)").count(),
        2,
        "audit + summary lines for the one cloud entry:\n{}",
        text
    );
    let content = std::fs::read_to_string(&cfg).unwrap();
    assert!(!content.contains("sk-live-key-must-go"), "cloud key stripped:\n{}", content);
    let doc: toml::Value = content.parse().unwrap();
    let providers = doc.get("providers").and_then(|p| p.as_array()).unwrap();
    let mm = providers
        .iter()
        .find(|t| t.get("id").and_then(|v| v.as_str()) == Some("minimax"))
        .expect("minimax entry preserved");
    assert_eq!(mm.get("enabled").and_then(|v| v.as_bool()), Some(false));
    assert!(mm.get("api_key").is_none());
}

/// Custom providers with external URLs are refused, never saved.
#[test]
fn test_setup_refuses_external_custom() {
    let (_dir, cfg, models) = temp_paths("refuse");
    let (ok, text) = run_setup(&cfg, &models, "y\nevil\nEvil\nhttps://api.evil.com/v1\n");
    assert!(ok, "setup must exit 0. output:\n{}", text);
    assert!(text.contains("local-only mode"), "output:\n{}", text);
    let content = std::fs::read_to_string(&cfg).unwrap();
    assert!(!content.contains("evil"), "refused entry not saved:\n{}", content);
}

/// `routerctl default` pins a connected model, shows it, rejects the rest.
#[test]
fn test_default_command_roundtrip() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in listener.incoming().take(1) {
            if let Ok(mut s) = stream {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let body = r#"{"data":[{"id":"mock-a"},{"id":"mock-b"}]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        }
    });
    let (_dir, cfg, models) = temp_paths("defcmd");
    let input = format!("y\nmockprov\nMock\nhttp://127.0.0.1:{}\n\n", port);
    let (ok, text) = run_setup(&cfg, &models, &input);
    assert!(ok, "setup must exit 0. output:\n{}", text);

    // No default yet: show says scoring decides.
    let (ok, text) = run_default(&cfg, None);
    assert!(ok, "default show must exit 0. output:\n{}", text);
    assert!(text.contains("scoring decides"), "output:\n{}", text);

    // Unknown model rejected.
    let (ok, text) = run_default(&cfg, Some("nope"));
    assert!(!ok, "unknown model must fail. output:\n{}", text);
    assert!(text.contains("not a connected model"), "output:\n{}", text);

    // Connected model pins both tiers.
    let (ok, text) = run_default(&cfg, Some("mock-b"));
    assert!(ok, "default set must exit 0. output:\n{}", text);
    let content = std::fs::read_to_string(&cfg).unwrap();
    assert_eq!(content.matches("default_model = \"mock-b\"").count(), 2, "both tiers pinned:\n{}", content);
    let (ok, text) = run_default(&cfg, None);
    assert!(ok && text.contains("mock-b"), "show prints default. output:\n{}", text);
    server.join().unwrap();
}
