use routerd_core::RouterConfig;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn get_routerctl_bin() -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_bin = manifest_dir.join("../target/debug/routerctl");
    if !target_bin.exists() {
        let _ = Command::new("cargo")
            .args(["build", "--bin", "routerctl"])
            .current_dir(&manifest_dir)
            .status();
    }
    if target_bin.exists() {
        return target_bin;
    }
    manifest_dir.join("../target/release/routerctl")
}

#[test]
fn test_routerctl_setup_end_to_end_wizard() {
    let routerctl_bin = get_routerctl_bin();

    let dir = tempdir().expect("Failed to create temporary directory");
    let config_path = dir.path().join("routerd.toml");
    let cred_path = dir.path().join("credentials.env");
    let models_dir = dir.path().join("models");

    // Inputs:
    // 1. MiniMax key: sk-minimax-test-key-123
    // 2. Mistral key: sk-mistral-test-key-456
    // 3. Custom provider: y, my-vllm, My Local vLLM, http://127.0.0.1:8000/v1, sk-vllm, vllm-model-a, vllm-model-b
    // 4. HF token: hf_synthetic_test_token_789
    // 5. Gemma 4 selection: 1, 3
    let stdin_payload = concat!(
        "sk-minimax-test-key-123\n",
        "sk-mistral-test-key-456\n",
        "y\n",
        "my-vllm\n",
        "My Local vLLM\n",
        "http://127.0.0.1:8000/v1\n",
        "sk-vllm\n",
        "vllm-model-a, vllm-model-b\n",
        "hf_synthetic_test_token_789\n",
        "1, 3\n"
    );

    let mut child = Command::new(routerctl_bin)
        .args([
            "setup",
            "--config",
            config_path.to_str().unwrap(),
            "--credentials-path",
            cred_path.to_str().unwrap(),
            "--models-dir",
            models_dir.to_str().unwrap(),
            "--no-reload",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn routerctl setup");

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_payload.as_bytes())
            .expect("Failed to write to stdin");
    }

    let output = child
        .wait_with_output()
        .expect("Failed to wait on routerctl setup");
    assert!(
        output.status.success(),
        "routerctl setup exited with error: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // 1. Verify credentials.env
    assert!(cred_path.exists());
    let cred_content = fs::read_to_string(&cred_path).unwrap();
    assert!(cred_content.contains("HF_TOKEN=hf_synthetic_test_token_789"));

    // 2. Verify model artifacts in models_dir
    assert!(models_dir.exists());
    let e2b_gguf = models_dir.join("gemma-4-e2b-it.gguf");
    let e2b_manifest = models_dir.join("gemma-4-e2b-it.manifest.json");
    let moe_gguf = models_dir.join("gemma-4-26b-a4b-it.gguf");
    let moe_manifest = models_dir.join("gemma-4-26b-a4b-it.manifest.json");

    assert!(e2b_gguf.exists(), "gemma-4-e2b-it.gguf must exist");
    assert!(e2b_manifest.exists(), "gemma-4-e2b-it.manifest.json must exist");
    assert!(moe_gguf.exists(), "gemma-4-26b-a4b-it.gguf must exist");
    assert!(moe_manifest.exists(), "gemma-4-26b-a4b-it.manifest.json must exist");

    // 3. Verify routerd.toml config
    assert!(config_path.exists());
    let config_content = fs::read_to_string(&config_path).unwrap();
    let cfg = RouterConfig::load_from_str(&config_content)
        .expect("Generated routerd.toml must parse cleanly");

    assert_eq!(cfg.thresholds.min_tokens_per_second, 10.0);
    assert!(config_content.contains("min_tokens_per_second"));

    let provider_ids: Vec<&str> = cfg.providers.iter().map(|p| p.id.as_str()).collect();
    assert!(provider_ids.contains(&"minimax"));
    assert!(provider_ids.contains(&"mistral"));
    assert!(provider_ids.contains(&"my-vllm"));
    assert!(provider_ids.contains(&"syntrop-local"));

    let minimax = cfg.providers.iter().find(|p| p.id == "minimax").unwrap();
    assert!(minimax.enabled);
    assert_eq!(minimax.api_key.as_deref(), Some("sk-minimax-test-key-123"));

    let mistral = cfg.providers.iter().find(|p| p.id == "mistral").unwrap();
    assert!(mistral.enabled);
    assert_eq!(mistral.api_key.as_deref(), Some("sk-mistral-test-key-456"));

    let vllm = cfg.providers.iter().find(|p| p.id == "my-vllm").unwrap();
    assert!(vllm.enabled);
    let vllm_models: Vec<&str> = vllm.models.iter().map(|m| m.name.as_str()).collect();
    assert!(vllm_models.contains(&"vllm-model-a"));
    assert!(vllm_models.contains(&"vllm-model-b"));

    let local = cfg.providers.iter().find(|p| p.id == "syntrop-local").unwrap();
    assert!(local.enabled);
    let local_models: Vec<&str> = local.models.iter().map(|m| m.name.as_str()).collect();
    assert!(local_models.contains(&"gemma-4-e2b-it"));
    assert!(local_models.contains(&"gemma-4-26b-a4b-it"));
}

#[test]
fn test_routerctl_setup_skip_leaves_disabled() {
    let routerctl_bin = get_routerctl_bin();

    let dir = tempdir().expect("Failed to create temporary directory");
    let config_path = dir.path().join("routerd.toml");
    let cred_path = dir.path().join("credentials.env");
    let models_dir = dir.path().join("models");

    // All empty / skipped
    let stdin_payload = "\n\nn\n\ns\n";

    let mut child = Command::new(routerctl_bin)
        .args([
            "setup",
            "--config",
            config_path.to_str().unwrap(),
            "--credentials-path",
            cred_path.to_str().unwrap(),
            "--models-dir",
            models_dir.to_str().unwrap(),
            "--no-reload",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn routerctl setup");

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_payload.as_bytes())
            .expect("Failed to write to stdin");
    }

    let output = child
        .wait_with_output()
        .expect("Failed to wait on routerctl setup");
    assert!(output.status.success());

    let config_content = fs::read_to_string(&config_path).unwrap();
    let cfg = RouterConfig::load_from_str(&config_content)
        .expect("Generated routerd.toml must parse cleanly");
    assert_eq!(cfg.thresholds.min_tokens_per_second, 10.0);
    assert!(config_content.contains("min_tokens_per_second"));

    let minimax = cfg.providers.iter().find(|p| p.id == "minimax").unwrap();
    assert!(!minimax.enabled, "MiniMax should be marked disabled when skipped");

    let mistral = cfg.providers.iter().find(|p| p.id == "mistral").unwrap();
    assert!(!mistral.enabled, "Mistral should be marked disabled when skipped");
}
