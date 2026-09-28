use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct VarlinkRequest {
    pub method: String,
    #[serde(default)]
    pub parameters: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VarlinkReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continues: Option<bool>,
}

impl VarlinkReply {
    fn to_bytes(&self) -> Vec<u8> {
        let mut b = serde_json::to_vec(self).unwrap();
        b.push(0);
        b
    }
}

#[test]
fn test_varlink_framing_and_nul_terminator() {
    let rep = VarlinkReply {
        parameters: Some(json!({ "status": "active" })),
        error: None,
        continues: Some(false),
    };
    let bytes = rep.to_bytes();
    assert_eq!(*bytes.last().unwrap(), 0u8);

    let clean = &bytes[..bytes.len() - 1];
    let parsed: VarlinkReply = serde_json::from_slice(clean).unwrap();
    assert_eq!(
        parsed.parameters.unwrap().get("status").unwrap(),
        "active"
    );
}

#[test]
fn test_varlink_error_frame() {
    let rep = VarlinkReply {
        parameters: Some(json!({ "parameter": "interface" })),
        error: Some("org.varlink.service.InvalidParameter".to_string()),
        continues: None,
    };
    let bytes = rep.to_bytes();
    assert_eq!(*bytes.last().unwrap(), 0u8);

    let parsed: VarlinkReply = serde_json::from_slice(&bytes[..bytes.len() - 1]).unwrap();
    assert_eq!(
        parsed.error.as_deref(),
        Some("org.varlink.service.InvalidParameter")
    );
}

#[tokio::test]
async fn test_varlink_server_e2e_communication() {
    use routerd_core::{ProviderConfig, ProviderModelConfig, RouterConfig, RouterEngine};
    use routerd_daemon::varlink::{bind_or_create_listener, run_varlink_listener};
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("io.syntrop.Router1");

    let mut config = RouterConfig::default();
    config.providers.push(ProviderConfig {
        id: "test-varlink-prov".to_string(),
        name: "Test Provider".to_string(),
        kind: "openai".to_string(),
        base_url: "http://127.0.0.1:9".to_string(),
        api_key: None,
        tier: "fast".to_string(),
        weight: 1.0,
        enabled: true,
        timeout_ms: 1000,
        models: vec![ProviderModelConfig {
            name: "test-varlink-model".to_string(),
            max_context_tokens: 4096,
            cost_per_input_token: 0.0000001,
            cost_per_output_token: 0.0000002,
            avg_latency_ms: 30.0,
            tokens_per_second: 150.0,
            tier: Some("fast".to_string()),
        }],
    });
    let engine = Arc::new(RouterEngine::new(config));

    let listener = bind_or_create_listener(&sock_path).unwrap();
    let eng_clone = engine.clone();
    tokio::spawn(async move {
        run_varlink_listener(listener, eng_clone).await;
    });

    // Connect client
    let stream = UnixStream::connect(&sock_path).await.unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    // 1. Call org.varlink.service.GetInfo
    let mut req1 = serde_json::to_vec(&json!({
        "method": "org.varlink.service.GetInfo"
    })).unwrap();
    req1.push(0);
    writer.write_all(&req1).await.unwrap();

    let mut buf = Vec::new();
    reader.read_until(0, &mut buf).await.unwrap();
    assert_eq!(buf.pop(), Some(0));
    let rep1: Value = serde_json::from_slice(&buf).unwrap();
    let params1 = rep1.get("parameters").unwrap();
    assert_eq!(params1.get("vendor").unwrap(), "syntropd");
    assert_eq!(params1.get("product").unwrap(), "routerd");

    // 2. Call io.syntrop.Router1.GetStatus
    let mut req2 = serde_json::to_vec(&json!({
        "method": "io.syntrop.Router1.GetStatus"
    })).unwrap();
    req2.push(0);
    writer.write_all(&req2).await.unwrap();

    buf.clear();
    reader.read_until(0, &mut buf).await.unwrap();
    assert_eq!(buf.pop(), Some(0));
    let rep2: Value = serde_json::from_slice(&buf).unwrap();
    let params2 = rep2.get("parameters").unwrap();
    assert_eq!(params2.get("status").unwrap(), "active");
    assert_eq!(params2.get("providers_count").unwrap(), 1);

    // 3. Call io.syntrop.Router1.ListModels
    let mut req3 = serde_json::to_vec(&json!({
        "method": "io.syntrop.Router1.ListModels"
    })).unwrap();
    req3.push(0);
    writer.write_all(&req3).await.unwrap();

    buf.clear();
    reader.read_until(0, &mut buf).await.unwrap();
    assert_eq!(buf.pop(), Some(0));
    let rep3: Value = serde_json::from_slice(&buf).unwrap();
    let models = rep3.get("parameters").unwrap().get("models").unwrap().as_array().unwrap();
    let model_names: Vec<&str> = models.iter().filter_map(|m| m.as_str()).collect();
    assert!(model_names.contains(&"test-varlink-model"));

    // 4. Call io.syntrop.Router1.RouteRequest
    let mut req4 = serde_json::to_vec(&json!({
        "method": "io.syntrop.Router1.RouteRequest",
        "parameters": {
            "tier": "fast",
            "estimated_tokens": 100
        }
    })).unwrap();
    req4.push(0);
    writer.write_all(&req4).await.unwrap();

    buf.clear();
    reader.read_until(0, &mut buf).await.unwrap();
    assert_eq!(buf.pop(), Some(0));
    let rep4: Value = serde_json::from_slice(&buf).unwrap();
    let candidates = rep4.get("parameters").unwrap().get("candidates").unwrap().as_array().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].get("provider_id").unwrap(), "test-varlink-prov");

    // 5. Call unknown method -> MethodNotFound
    let mut req5 = serde_json::to_vec(&json!({
        "method": "io.syntrop.Router1.NonExistent"
    })).unwrap();
    req5.push(0);
    writer.write_all(&req5).await.unwrap();

    buf.clear();
    reader.read_until(0, &mut buf).await.unwrap();
    assert_eq!(buf.pop(), Some(0));
    let rep5: Value = serde_json::from_slice(&buf).unwrap();
    assert_eq!(rep5.get("error").unwrap(), "org.varlink.service.MethodNotFound");
}
