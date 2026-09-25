use anyhow::{anyhow, Result};
use reqwest::Client as HttpClient;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

const RPC_TIMEOUT: Duration = Duration::from_secs(5);

pub struct RouterctlClient {
    socket_path: PathBuf,
    http_url: String,
    http_client: HttpClient,
}

impl RouterctlClient {
    pub fn new(socket_path: impl Into<PathBuf>, http_url: impl Into<String>) -> Self {
        Self {
            socket_path: socket_path.into(),
            http_url: http_url.into().trim_end_matches('/').to_string(),
            http_client: HttpClient::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    pub async fn varlink_call(&self, method: &str, params: Value) -> Result<Value> {
        let stream = timeout(RPC_TIMEOUT, UnixStream::connect(&self.socket_path))
            .await
            .map_err(|_| anyhow!("Connection to Varlink socket {:?} timed out", self.socket_path))?
            .map_err(|e| anyhow!("Failed to connect to {:?}: {}", self.socket_path, e))?;

        let (mut reader, mut writer) = stream.into_split();

        let req = json!({
            "method": method,
            "parameters": params
        });

        let mut req_bytes = serde_json::to_vec(&req)?;
        req_bytes.push(0);

        timeout(RPC_TIMEOUT, writer.write_all(&req_bytes))
            .await
            .map_err(|_| anyhow!("Varlink write timed out"))??;

        let mut buf = Vec::with_capacity(1024);
        let mut byte = [0u8; 1];

        let read_future = async {
            loop {
                let n = reader.read(&mut byte).await?;
                if n == 0 || byte[0] == 0 {
                    break;
                }
                buf.push(byte[0]);
            }
            Ok::<(), std::io::Error>(())
        };

        timeout(RPC_TIMEOUT, read_future)
            .await
            .map_err(|_| anyhow!("Varlink read timed out"))??;

        if buf.is_empty() {
            return Err(anyhow!("Empty reply from routerd"));
        }

        let resp: Value = serde_json::from_slice(&buf)?;
        if let Some(err) = resp.get("error").and_then(|e| e.as_str()) {
            return Err(anyhow!("Varlink error: {}", err));
        }

        Ok(resp.get("parameters").cloned().unwrap_or(Value::Null))
    }

    pub async fn get_status(&self) -> Result<Value> {
        if self.socket_path.exists() {
            if let Ok(val) = self.varlink_call("io.syntrop.Router1.GetStatus", json!({})).await {
                return Ok(val);
            }
        }

        // Fallback to HTTP health
        let url = format!("{}/health", self.http_url);
        let resp = self.http_client.get(&url).send().await?;
        let json_val = resp.json().await?;
        Ok(json_val)
    }

    pub async fn list_providers(&self) -> Result<Value> {
        self.varlink_call("io.syntrop.Router1.ListProviders", json!({})).await
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        if self.socket_path.exists() {
            if let Ok(val) = self.varlink_call("io.syntrop.Router1.ListModels", json!({})).await {
                if let Some(arr) = val.get("models").and_then(|m| m.as_array()) {
                    let mut list = Vec::new();
                    for item in arr {
                        if let Some(s) = item.as_str() {
                            list.push(s.to_string());
                        }
                    }
                    return Ok(list);
                }
            }
        }

        // Fallback to HTTP /v1/models
        let url = format!("{}/v1/models", self.http_url);
        let resp = self.http_client.get(&url).send().await?;
        let json_val: Value = resp.json().await?;
        let mut list = Vec::new();
        if let Some(arr) = json_val.get("data").and_then(|d| d.as_array()) {
            for item in arr {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    list.push(id.to_string());
                }
            }
        }
        Ok(list)
    }

    pub async fn route_request(
        &self,
        model: Option<&str>,
        tier: Option<&str>,
        tokens: Option<usize>,
        require_stream: bool,
    ) -> Result<Value> {
        let params = json!({
            "model": model,
            "tier": tier,
            "estimated_tokens": tokens,
            "require_stream": require_stream
        });
        self.varlink_call("io.syntrop.Router1.RouteRequest", params).await
    }

    pub async fn test_provider(&self, provider_id: &str) -> Result<Value> {
        let params = json!({
            "provider_id": provider_id
        });
        self.varlink_call("io.syntrop.Router1.TestProvider", params).await
    }

    pub async fn test_benchmark(
        &self,
        model: &str,
        prompt: &str,
        tier: Option<&str>,
    ) -> Result<(f64, String)> {
        let start = std::time::Instant::now();
        let url = format!("{}/v1/chat/completions", self.http_url);

        let mut body = json!({
            "model": model,
            "messages": [
                { "role": "user", "content": prompt }
            ],
            "max_tokens": 64
        });
        if let Some(t) = tier {
            body["tier"] = json!(t);
        }

        let resp = self.http_client.post(&url).json(&body).send().await?;
        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

        if !resp.status().is_success() {
            let err_txt = resp.text().await.unwrap_or_default();
            return Err(anyhow!("HTTP error: {}", err_txt));
        }

        let val: Value = resp.json().await?;
        let content = val
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c0| c0.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|txt| txt.as_str())
            .unwrap_or("")
            .to_string();

        Ok((latency_ms, content))
    }

    pub async fn get_info(&self) -> Result<Value> {
        self.varlink_call("org.varlink.service.GetInfo", json!({})).await
    }

    pub async fn get_interface_description(&self, iface: &str) -> Result<String> {
        let val = self
            .varlink_call(
                "org.varlink.service.GetInterfaceDescription",
                json!({ "interface": iface }),
            )
            .await?;
        let desc = val
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_string();
        Ok(desc)
    }
}
