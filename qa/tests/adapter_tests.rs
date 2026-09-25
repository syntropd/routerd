use routerd_core::adapters::create_adapter;
use routerd_core::config::ProviderConfig;
use routerd_core::models::ProviderModelConfig;

#[tokio::test]
async fn test_adapter_factory_instantiation() {
    let p_openai = ProviderConfig {
        id: "groq".to_string(),
        name: "Groq LPU".to_string(),
        kind: "openai".to_string(),
        base_url: "https://api.groq.com/openai/v1".to_string(),
        api_key: Some("gsk_test".to_string()),
        tier: "fast".to_string(),
        weight: 1.2,
        enabled: true,
        timeout_ms: 15000,
        models: vec![ProviderModelConfig {
            name: "llama-3.3-70b-versatile".to_string(),
            max_context_tokens: 128000,
            cost_per_input_token: 0.00000059,
            cost_per_output_token: 0.00000079,
            avg_latency_ms: 120.0,
            tokens_per_second: 280.0,
            tier: Some("fast".to_string()),
        }],
    };
    let adapter_openai = create_adapter(&p_openai);
    let models = adapter_openai.list_models().await.unwrap();
    assert_eq!(models, vec!["llama-3.3-70b-versatile"]);

    let p_minimax = ProviderConfig {
        id: "minimax".to_string(),
        name: "MiniMax AI".to_string(),
        kind: "minimax".to_string(),
        base_url: "https://api.minimaxi.chat/v1".to_string(),
        api_key: Some("mm_test".to_string()),
        tier: "hard".to_string(),
        weight: 1.0,
        enabled: true,
        timeout_ms: 30000,
        models: vec![ProviderModelConfig {
            name: "abab6.5s-chat".to_string(),
            max_context_tokens: 245760,
            cost_per_input_token: 0.000001,
            cost_per_output_token: 0.000001,
            avg_latency_ms: 350.0,
            tokens_per_second: 80.0,
            tier: Some("hard".to_string()),
        }],
    };
    let adapter_minimax = create_adapter(&p_minimax);
    let mm_models = adapter_minimax.list_models().await.unwrap();
    assert_eq!(mm_models, vec!["abab6.5s-chat"]);

    let p_ollama = ProviderConfig {
        id: "ollama-lan".to_string(),
        name: "Ollama LAN".to_string(),
        kind: "ollama".to_string(),
        base_url: "http://192.168.1.100:11434".to_string(),
        api_key: None,
        tier: "fast".to_string(),
        weight: 1.1,
        enabled: true,
        timeout_ms: 20000,
        models: vec![ProviderModelConfig {
            name: "qwen2.5-coder:7b".to_string(),
            max_context_tokens: 32768,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            avg_latency_ms: 95.0,
            tokens_per_second: 110.0,
            tier: Some("fast".to_string()),
        }],
    };
    let adapter_ollama = create_adapter(&p_ollama);
    let ollama_models = adapter_ollama.list_models().await.unwrap();
    assert_eq!(ollama_models, vec!["qwen2.5-coder:7b"]);

    let p_varlink = ProviderConfig {
        id: "syntrop-local".to_string(),
        name: "Syntrop Local".to_string(),
        kind: "varlink".to_string(),
        base_url: "/run/syntrop/io.syntrop.Inference1".to_string(),
        api_key: None,
        tier: "fast".to_string(),
        weight: 1.3,
        enabled: true,
        timeout_ms: 10000,
        models: vec![ProviderModelConfig {
            name: "syntrop-local-qwen".to_string(),
            max_context_tokens: 16384,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            avg_latency_ms: 40.0,
            tokens_per_second: 300.0,
            tier: Some("fast".to_string()),
        }],
    };
    let adapter_varlink = create_adapter(&p_varlink);
    let varlink_models = adapter_varlink.list_models().await.unwrap();
    assert_eq!(varlink_models, vec!["syntrop-local-qwen"]);
}
