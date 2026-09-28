use super::{PressureLevel, PressureMetrics};
use crate::error::{Result, RouterError};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::RwLock;
use tokio::time::timeout;
use tracing::debug;

const RPC_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct TelemetryClient {
    socket_path: PathBuf,
    cache: Arc<RwLock<Option<(PressureMetrics, Instant)>>>,
    cache_ttl: Duration,
}

impl TelemetryClient {
    pub fn new<P: Into<PathBuf>>(socket_path: P) -> Self {
        Self {
            socket_path: socket_path.into(),
            cache: Arc::new(RwLock::new(None)),
            cache_ttl: Duration::from_millis(1500),
        }
    }

    /// Read pressure metrics, using cached value if fresh or querying inferenced Varlink / kernel PSI.
    pub async fn get_pressure(&self) -> PressureMetrics {
        // Check cache first
        {
            let guard = self.cache.read().await;
            if let Some((metrics, timestamp)) = &*guard {
                if timestamp.elapsed() < self.cache_ttl {
                    return metrics.clone();
                }
            }
        }

        // Try Varlink socket
        let metrics = match self.query_varlink_pressure().await {
            Ok(m) => m,
            Err(_) => {
                // Fall back to reading kernel PSI or env var simulation
                TelemetryClient::read_kernel_or_simulated_psi()
            }
        };

        // Update cache
        {
            let mut guard = self.cache.write().await;
            *guard = Some((metrics.clone(), Instant::now()));
        }

        metrics
    }

    async fn query_varlink_pressure(&self) -> Result<PressureMetrics> {
        if !self.socket_path.exists() {
            return Err(RouterError::Varlink("Inferenced socket does not exist".into()));
        }

        let stream = timeout(RPC_TIMEOUT, UnixStream::connect(&self.socket_path))
            .await
            .map_err(|_| RouterError::Timeout("Inferenced connect timed out".into()))?
            .map_err(|e| RouterError::Varlink(format!("Failed to connect to inferenced: {}", e)))?;

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let req = json!({
            "method": "io.syntrop.Inference1.GetPressure",
            "parameters": {}
        });
        let mut req_bytes = serde_json::to_vec(&req)?;
        req_bytes.push(0); // NUL terminator for Varlink

        timeout(RPC_TIMEOUT, writer.write_all(&req_bytes))
            .await
            .map_err(|_| RouterError::Timeout("Varlink write timed out".into()))??;

        let mut buf = Vec::with_capacity(512);

        timeout(RPC_TIMEOUT, reader.read_until(0, &mut buf))
            .await
            .map_err(|_| RouterError::Timeout("Varlink read timed out".into()))??;

        if buf.last() == Some(&0) {
            buf.pop();
        }

        if buf.is_empty() {
            return Err(RouterError::Varlink("Empty Varlink response from inferenced".into()));
        }

        let resp: Value = serde_json::from_slice(&buf)?;
        if let Some(err) = resp.get("error").and_then(|e| e.as_str()) {
            return Err(RouterError::Varlink(format!("Inferenced error reply: {}", err)));
        }

        let params = resp.get("parameters").cloned().unwrap_or(Value::Null);
        let level_str = params
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("Normal");

        let level = match level_str.to_ascii_lowercase().as_str() {
            "critical" => PressureLevel::Critical,
            "elevated" => PressureLevel::Elevated,
            _ => PressureLevel::Normal,
        };

        let mem_some = params
            .get("memory_some")
            .and_then(|v| v.as_f64())
            .map(|f| f as f32)
            .unwrap_or(0.0);

        let cpu_some = params
            .get("cpu_some")
            .and_then(|v| v.as_f64())
            .map(|f| f as f32)
            .unwrap_or(0.0);

        let io_some = params
            .get("io_some")
            .and_then(|v| v.as_f64())
            .map(|f| f as f32)
            .unwrap_or(0.0);

        debug!(
            "Queried inferenced PSI via Varlink: level={:?}, mem_some={}",
            level, mem_some
        );

        Ok(PressureMetrics {
            level,
            memory_some_avg10: mem_some,
            memory_full_avg10: 0.0,
            cpu_some_avg10: cpu_some,
            io_some_avg10: io_some,
            source: "varlink:io.syntrop.Inference1".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_socket_falls_back_and_caches() {
        let client = TelemetryClient::new("/tmp/qa-nope-psi/never.sock");
        let m = client.get_pressure().await;
        assert!(!m.source.is_empty());
        assert_ne!(m.source, "varlink:io.syntrop.Inference1");
        let cached = client.get_pressure().await;
        assert_eq!(cached.source, m.source);
    }
}
