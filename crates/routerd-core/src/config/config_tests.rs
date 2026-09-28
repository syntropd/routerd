use super::{is_local_provider, RouterConfig};

#[test]
fn test_parse_sample_config() {
    let toml_str = r#"
    [daemon]
    listen_tcp = "127.0.0.1:32768"

    [tiers.fast]
    latency_weight = 0.7
    cost_weight = 0.2
    capability_weight = 0.1

    [[providers]]
    id = "test-provider"
    kind = "openai"
    base_url = "https://api.example.com/v1"
    tier = "fast"

    [[providers.models]]
    name = "test-model"
    max_context_tokens = 64000
    "#;

    let cfg = RouterConfig::load_from_str(toml_str).unwrap();
    assert_eq!(cfg.daemon.listen_tcp, "127.0.0.1:32768");
    assert_eq!(cfg.thresholds.min_tokens_per_second, 10.0);
    assert_eq!(cfg.providers.len(), 1);
    assert_eq!(cfg.providers[0].id, "test-provider");
    assert_eq!(cfg.providers[0].models[0].name, "test-model");
    assert_eq!(cfg.providers[0].models[0].max_context_tokens, 64000);
}

#[test]
fn test_thresholds_custom_min_tokens_per_second() {
    let toml_str = r#"
    [thresholds]
    min_tokens_per_second = 25.5
    "#;
    let cfg = RouterConfig::load_from_str(toml_str).unwrap();
    assert_eq!(cfg.thresholds.min_tokens_per_second, 25.5);
}

#[test]
fn test_local_provider_rule() {
    // Owned engine, varlink broker, localhost APIs.
    assert!(is_local_provider("runtimed", "/run/syntrop/io.syntrop.Runtime1"));
    assert!(is_local_provider("varlink", "/run/syntrop/io.syntrop.Inference1"));
    assert!(is_local_provider("openai", "http://127.0.0.1:8000/v1"));
    assert!(is_local_provider("openai", "http://localhost:8000/v1"));
    assert!(is_local_provider("openai", "http://[::1]:8000/v1"));
    // Cloud APIs refused even for openai-kind entries.
    assert!(!is_local_provider("openai", "https://api.groq.com/openai/v1"));
    assert!(!is_local_provider("minimax", "https://api.minimaxi.chat/v1"));
    assert!(!is_local_provider("openai", "https://127.0.0.1.evil.com/v1"));
    assert!(!is_local_provider("", ""));
}

#[test]
fn test_server_and_listen_socket_aliases() {
    let toml_str = r#"
    [server]
    listen_tcp = "127.0.0.1:39999"
    listen_socket = "/run/syntrop/custom-router.sock"
    "#;
    let cfg = RouterConfig::load_from_str(toml_str).unwrap();
    assert_eq!(cfg.daemon.listen_tcp, "127.0.0.1:39999");
    assert_eq!(cfg.daemon.listen_unix, "/run/syntrop/custom-router.sock");
}
