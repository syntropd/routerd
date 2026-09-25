use crate::error::{Result, RouterError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::RwLock;
use tokio::time::timeout;
use tracing::debug;

pub const DEFAULT_INFERENCED_SOCKET: &str = "/run/syntrop/io.syntrop.Inference1";
const RPC_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PressureLevel {
    Normal,
    Elevated,
    Critical,
}

impl Default for PressureLevel {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PressureMetrics {
    pub level: PressureLevel,
    pub memory_some_avg10: f32,
    pub memory_full_avg10: f32,
    pub cpu_some_avg10: f32,
    pub io_some_avg10: f32,
    pub source: String,
}

impl Default for PressureMetrics {
    fn default() -> Self {
        Self {
            level: PressureLevel::Normal,
            memory_some_avg10: 0.0,
            memory_full_avg10: 0.0,
            cpu_some_avg10: 0.0,
            io_some_avg10: 0.0,
            source: "default".to_string(),
        }
    }
}

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
                Self::read_kernel_or_simulated_psi()
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

        let (mut reader, mut writer) = stream.into_split();

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
            .map_err(|_| RouterError::Timeout("Varlink read timed out".into()))??;

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

    /// Read Linux PSI from `/proc/pressure/memory` or check `INFERENCED_SIMULATE_PSI`.
    pub fn read_kernel_or_simulated_psi() -> PressureMetrics {
        if let Ok(sim) = std::env::var("INFERENCED_SIMULATE_PSI") {
            let sim_lower = sim.to_ascii_lowercase();
            if sim_lower == "critical" {
                return PressureMetrics {
                    level: PressureLevel::Critical,
                    memory_some_avg10: 65.0,
                    memory_full_avg10: 30.0,
                    cpu_some_avg10: 20.0,
                    io_some_avg10: 10.0,
                    source: "simulated:critical".to_string(),
                };
            } else if sim_lower == "elevated" {
                return PressureMetrics {
                    level: PressureLevel::Elevated,
                    memory_some_avg10: 25.0,
                    memory_full_avg10: 5.0,
                    cpu_some_avg10: 15.0,
                    io_some_avg10: 5.0,
                    source: "simulated:elevated".to_string(),
                };
            }
        }

        let psi_path = Path::new("/proc/pressure/memory");
        if psi_path.exists() {
            if let Ok(content) = fs::read_to_string(psi_path) {
                let mut mem_some = 0.0f32;
                let mut mem_full = 0.0f32;
                for line in content.lines() {
                    if line.starts_with("some ") {
                        if let Some(avg10) = parse_psi_avg10(line) {
                            mem_some = avg10;
                        }
                    } else if line.starts_with("full ") {
                        if let Some(avg10) = parse_psi_avg10(line) {
                            mem_full = avg10;
                        }
                    }
                }

                let level = if mem_some >= 40.0 || mem_full >= 20.0 {
                    PressureLevel::Critical
                } else if mem_some >= 15.0 || mem_full >= 5.0 {
                    PressureLevel::Elevated
                } else {
                    PressureLevel::Normal
                };

                return PressureMetrics {
                    level,
                    memory_some_avg10: mem_some,
                    memory_full_avg10: mem_full,
                    cpu_some_avg10: 0.0,
                    io_some_avg10: 0.0,
                    source: "kernel:/proc/pressure/memory".to_string(),
                };
            }
        }

        PressureMetrics::default()
    }
}

fn parse_psi_avg10(line: &str) -> Option<f32> {
    for part in line.split_whitespace() {
        if let Some(val_str) = part.strip_prefix("avg10=") {
            return val_str.parse::<f32>().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_psi_parsing() {
        let sample = "some avg10=18.42 avg60=12.11 avg300=5.00 total=1234567\nfull avg10=2.30 avg60=1.10 avg300=0.50 total=45678";
        let mut some_val = 0.0;
        let mut full_val = 0.0;
        for line in sample.lines() {
            if line.starts_with("some ") {
                some_val = parse_psi_avg10(line).unwrap();
            } else if line.starts_with("full ") {
                full_val = parse_psi_avg10(line).unwrap();
            }
        }
        assert_eq!(some_val, 18.42);
        assert_eq!(full_val, 2.30);
    }
}
