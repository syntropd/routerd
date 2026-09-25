use routerd_core::credentials::resolve_credential;
use routerd_core::RouterConfig;
use std::env;
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn test_template_config_parsing() {
    let template = include_str!("../../systemd/routerd.toml");
    let cfg = RouterConfig::load_from_str(template).expect("Default routerd.toml must parse cleanly");

    assert_eq!(cfg.daemon.listen_tcp, "127.0.0.1:32768");
    assert_eq!(cfg.daemon.varlink_socket, "/run/syntrop/io.syntrop.Router1");
    assert_eq!(cfg.providers.len(), 8);

    let provider_ids: Vec<&str> = cfg.providers.iter().map(|p| p.id.as_str()).collect();
    assert!(provider_ids.contains(&"minimax"));
    assert!(provider_ids.contains(&"groq"));
    assert!(provider_ids.contains(&"gemini"));
    assert!(provider_ids.contains(&"mistral"));
    assert!(provider_ids.contains(&"devin"));
    assert!(provider_ids.contains(&"ollama-lan-1"));
    assert!(provider_ids.contains(&"ollama-lan-2"));
    assert!(provider_ids.contains(&"syntrop-local"));

    assert!(cfg.tiers.contains_key("fast"));
    assert!(cfg.tiers.contains_key("hard"));
}

#[test]
fn test_credential_resolution_schemes() {
    env::set_var("GROQ_API_KEY", "gsk_test_key_abc123");
    let res = resolve_credential(Some("env:GROQ_API_KEY"), "groq").unwrap();
    assert_eq!(res, Some("gsk_test_key_abc123".to_string()));

    let res_sub = resolve_credential(Some("${GROQ_API_KEY}"), "groq").unwrap();
    assert_eq!(res_sub, Some("gsk_test_key_abc123".to_string()));

    let mut tmp = NamedTempFile::new().unwrap();
    write!(tmp, " file_secret_key_xyz \n").unwrap();
    let res_file = resolve_credential(Some(tmp.path().to_str().unwrap()), "custom").unwrap();
    assert_eq!(res_file, Some("file_secret_key_xyz".to_string()));
}
