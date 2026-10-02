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

        let mut req = json!({
            "method": "io.syntrop.Telemetry1.GetKernelPressure",
            "parameters": {}
        });
        let mut req_bytes = serde_json::to_vec(&req)?;
        req_bytes.push(0);

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

        let mut resp: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
        let mut is_telemetry1 = true;
        if resp.get("error").is_some() {
            // Fallback to io.syntrop.Inference1.GetPressure
            req = json!({
                "method": "io.syntrop.Inference1.GetPressure",
                "parameters": {}
            });
            req_bytes = serde_json::to_vec(&req)?;
            req_bytes.push(0);
            timeout(RPC_TIMEOUT, writer.write_all(&req_bytes))
                .await
                .map_err(|_| RouterError::Timeout("Varlink write timed out".into()))??;

            buf.clear();
            timeout(RPC_TIMEOUT, reader.read_until(0, &mut buf))
                .await
                .map_err(|_| RouterError::Timeout("Varlink read timed out".into()))??;
            if buf.last() == Some(&0) {
                buf.pop();
            }
            resp = serde_json::from_slice(&buf)?;
            is_telemetry1 = false;
        }

        if let Some(err) = resp.get("error").and_then(|e| e.as_str()) {
            return Err(RouterError::Varlink(format!("Inferenced error reply: {}", err)));
        }

        let params = resp.get("parameters").cloned().unwrap_or(Value::Null);
        let mem_some = params.get("memory_some").and_then(|v| v.as_f64()).map(|f| f as f32).unwrap_or(0.0);
        let mem_full = params.get("memory_full").and_then(|v| v.as_f64()).map(|f| f as f32).unwrap_or(0.0);
        let cpu_some = params.get("cpu_some").and_then(|v| v.as_f64()).map(|f| f as f32).unwrap_or(0.0);
        let io_some = params.get("io_some").and_then(|v| v.as_f64()).map(|f| f as f32).unwrap_or(0.0);
        let runqueue_latency_us = params.get("runqueue_latency_us").and_then(|v| v.as_u64()).unwrap_or(0);
        let ebpf_active = params.get("ebpf_active").and_then(|v| v.as_bool()).unwrap_or(false);

        let level = if mem_full > 10.0 || mem_some > 40.0 || io_some > 50.0 || runqueue_latency_us > 100_000 {
            PressureLevel::Critical
        } else if mem_some > 15.0 || io_some > 20.0 || cpu_some > 60.0 || runqueue_latency_us > 40_000 {
            PressureLevel::Elevated
        } else {
            PressureLevel::Normal
        };

        let source = if is_telemetry1 {
            "varlink:io.syntrop.Telemetry1".to_string()
        } else {
            "varlink:io.syntrop.Inference1".to_string()
        };

        debug!("Queried PSI via Varlink: level={:?}, mem_some={}, source={}", level, mem_some, source);

        Ok(PressureMetrics {
            level,
            memory_some_avg10: mem_some,
            memory_full_avg10: mem_full,
            cpu_some_avg10: cpu_some,
            io_some_avg10: io_some,
            runqueue_latency_us,
            ebpf_active,
            source,
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
        assert_ne!(m.source, "varlink:io.syntrop.Telemetry1");
        let cached = client.get_pressure().await;
        assert_eq!(cached.source, m.source);
    }
}
