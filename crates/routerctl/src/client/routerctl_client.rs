use anyhow::{anyhow, Result};
use reqwest::Client as HttpClient;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::timeout;

const RPC_TIMEOUT: Duration = Duration::from_secs(5);

pub struct RouterctlClient {
    pub(super) socket_path: PathBuf,
    pub(super) http_url: String,
    pub(super) http_client: HttpClient,
}

impl RouterctlClient {
    pub fn new(socket_path: impl Into<PathBuf>, http_url: impl Into<String>) -> Self {
        Self {
            socket_path: socket_path.into(),
            http_url: http_url.into().trim_end_matches('/').to_string(),
            http_client: HttpClient::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
        }
    }

    pub async fn varlink_call(&self, method: &str, params: Value) -> Result<Value> {
        Self::varlink_call_path(&self.socket_path, method, params, RPC_TIMEOUT).await
    }

    /// One `\0`-framed Varlink call against any socket path, with an
    /// explicit per-hop budget. Slow engine calls (model warmup) pass a
    /// generous one; snappy router probes keep `RPC_TIMEOUT`.
    pub async fn varlink_call_path(
        socket_path: &PathBuf,
        method: &str,
        params: Value,
        budget: Duration,
    ) -> Result<Value> {
        let stream = timeout(budget, UnixStream::connect(socket_path))
            .await
            .map_err(|_| anyhow!("Connection to Varlink socket {:?} timed out", socket_path))?
            .map_err(|e| anyhow!("Failed to connect to {:?}: {}", socket_path, e))?;

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let req = json!({
            "method": method,
            "parameters": params
        });

        let mut req_bytes = serde_json::to_vec(&req)?;
        req_bytes.push(0);

        timeout(budget, writer.write_all(&req_bytes))
            .await
            .map_err(|_| anyhow!("Varlink write timed out"))??;

        let mut buf = Vec::with_capacity(1024);
        timeout(budget, reader.read_until(0, &mut buf))
            .await
            .map_err(|_| anyhow!("Varlink read timed out"))??;

        if buf.last() == Some(&0) {
            buf.pop();
        }

        if buf.is_empty() {
            return Err(anyhow!("Empty reply"));
        }

        let resp: Value = serde_json::from_slice(&buf)?;
        if let Some(err) = resp.get("error").and_then(|e| e.as_str()) {
            return Err(anyhow!("Varlink error: {}", err));
        }

        Ok(resp.get("parameters").cloned().unwrap_or(Value::Null))
    }
}
