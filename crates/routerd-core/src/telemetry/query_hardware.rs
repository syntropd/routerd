use super::hardware_types::*;
use crate::error::{Result, RouterError};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::RwLock;
use tokio::time::timeout;

const RPC_TIMEOUT: Duration = Duration::from_millis(500);

/// Queries inferenced and runtimed concurrently to assemble hardware telemetry.
#[derive(Debug, Clone)]
pub struct HardwareTelemetryClient {
    inferenced_socket: PathBuf,
    runtimed_socket: PathBuf,
    cache: Arc<RwLock<Option<(HardwareTelemetryReport, Instant)>>>,
    cache_ttl: Duration,
}

impl HardwareTelemetryClient {
    pub fn new<P1: Into<PathBuf>, P2: Into<PathBuf>>(inferenced_sock: P1, runtimed_sock: P2) -> Self {
        Self {
            inferenced_socket: inferenced_sock.into(),
            runtimed_socket: runtimed_sock.into(),
            cache: Arc::new(RwLock::new(None)),
            cache_ttl: Duration::from_millis(1500),
        }
    }

    pub async fn get_report(&self) -> HardwareTelemetryReport {
        {
            let guard = self.cache.read().await;
            if let Some((report, timestamp)) = &*guard {
                if timestamp.elapsed() < self.cache_ttl {
                    return report.clone();
                }
            }
        }

        let (topo_res, leases_res, comp_leases_res, load_res) = tokio::join!(
            Self::query_varlink_call(&self.inferenced_socket, "io.syntrop.Inference1.GetTopology"),
            Self::query_varlink_call(&self.inferenced_socket, "io.syntrop.Inference1.ListLeases"),
            Self::query_varlink_call(&self.inferenced_socket, "io.syntrop.Inference1.ListCompositeLeases"),
            Self::query_varlink_call(&self.runtimed_socket, "io.syntrop.Runtime1.GetLoad"),
        );

        let report = Self::assemble_report(
            topo_res.ok(),
            leases_res.ok(),
            comp_leases_res.ok(),
            load_res.ok(),
        );

        {
            let mut guard = self.cache.write().await;
            *guard = Some((report.clone(), Instant::now()));
        }

        report
    }

    async fn query_varlink_call(socket_path: &Path, method: &str) -> Result<Value> {
        if !socket_path.exists() {
            return Err(RouterError::Varlink(format!("Socket {:?} does not exist", socket_path)));
        }

        let stream = timeout(RPC_TIMEOUT, UnixStream::connect(socket_path))
            .await
            .map_err(|_| RouterError::Timeout(format!("Connect to {:?} timed out", socket_path)))?
            .map_err(|e| RouterError::Varlink(format!("Connect error {:?}: {}", socket_path, e)))?;

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let req = json!({ "method": method, "parameters": {} });
        let mut req_bytes = serde_json::to_vec(&req)?;
        req_bytes.push(0);

        timeout(RPC_TIMEOUT, writer.write_all(&req_bytes))
            .await
            .map_err(|_| RouterError::Timeout("Varlink write timed out".into()))??;

        let mut buf = Vec::with_capacity(1024);
        timeout(RPC_TIMEOUT, reader.read_until(0, &mut buf))
            .await
            .map_err(|_| RouterError::Timeout("Varlink read timed out".into()))??;

        if buf.last() == Some(&0) {
            buf.pop();
        }
        if buf.is_empty() {
            return Err(RouterError::Varlink("Empty Varlink response".into()));
        }

        let resp: Value = serde_json::from_slice(&buf)?;
        if let Some(err) = resp.get("error").and_then(|e| e.as_str()) {
            return Err(RouterError::Varlink(format!("Varlink error {}: {}", method, err)));
        }

        Ok(resp.get("parameters").cloned().unwrap_or(Value::Null))
    }

    fn assemble_report(
        topo_val: Option<Value>,
        leases_val: Option<Value>,
        comp_leases_val: Option<Value>,
        load_val: Option<Value>,
    ) -> HardwareTelemetryReport {
        let psi = super::TelemetryClient::read_kernel_or_simulated_psi();
        let mut gpus = Vec::new();
        let mut host_ram = HostRamTelemetry::default();
        let mut cpu = CpuMatrixTelemetry::default();
        let mut active_leases = Vec::new();
        let mut composite_leases = Vec::new();

        if let Some(t) = topo_val {
            if let Some(planes) = t.get("planes").and_then(|p| p.as_array()) {
                for p in planes {
                    let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let kind = p.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                    let total_mem = p.get("total_memory").and_then(|v| v.as_u64()).unwrap_or(0);
                    let avail_mem = p.get("available_memory").and_then(|v| v.as_u64()).unwrap_or(0);
                    let used_mem = total_mem.saturating_sub(avail_mem);
                    let kernel_used_memory = p.get("kernel_used_memory").and_then(|v| v.as_u64()).unwrap_or(0);
                    let p2p_links = p.get("p2p_links").and_then(|v| serde_json::from_value(v.clone()).ok());
                    let features = p
                        .get("features")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|s| s.as_str().map(|x| x.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();

                    if kind == "CpuMatrixExtension" || id == "cpu-host" {
                        cpu.name = name;
                        cpu.features = features;
                    } else {
                        gpus.push(DrmGpuTelemetry {
                            id,
                            name,
                            total_vram_bytes: total_mem,
                            used_vram_bytes: used_mem,
                            available_vram_bytes: avail_mem,
                            features,
                            p2p_links,
                            kernel_used_memory,
                        });
                    }
                }
            }
            host_ram.total_bytes = t.get("total_ram").and_then(|v| v.as_u64()).unwrap_or(0);
            host_ram.available_bytes = t.get("available_ram").and_then(|v| v.as_u64()).unwrap_or(0);
            host_ram.used_bytes = host_ram.total_bytes.saturating_sub(host_ram.available_bytes);
            let cores = t.get("cpu_cores").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if cores > 0 {
                cpu.cores = cores;
            }
        }

        if cpu.cores == 0 {
            cpu.cores = std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(1);
        }
        if cpu.name.is_empty() {
            cpu.name = "Host CPU Matrix Accelerator".to_string();
        }

        if let Some(l) = leases_val {
            if let Some(leases_arr) = l.get("leases").and_then(|v| v.as_array()) {
                for item in leases_arr {
                    active_leases.push(ComputeLeaseTelemetry {
                        id: item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        plane_id: item.get("plane_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        allocated_memory: item.get("allocated_memory").and_then(|v| v.as_u64()).unwrap_or(0),
                        priority: item.get("priority").and_then(|v| v.as_str()).unwrap_or("Interactive").to_string(),
                        state: item.get("state").and_then(|v| v.as_str()).unwrap_or("Active").to_string(),
                        client_unit: item.get("client_unit").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        client_pid: item.get("client_pid").and_then(|v| v.as_u64()).map(|u| u as u32),
                    });
                }
            }
        }

        if let Some(cl) = comp_leases_val {
            let key = if cl.get("composite_leases").is_some() { "composite_leases" } else { "leases" };
            if let Some(arr) = cl.get(key).and_then(|v| v.as_array()) {
                for item in arr {
                    let slices = item.get("slices").and_then(|v| v.as_array()).map(|s_arr| {
                        s_arr.iter().map(|s| CompositeSliceTelemetry {
                            plane_id: s.get("plane_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            allocated_memory: s.get("allocated_memory").and_then(|v| v.as_u64()).unwrap_or(0),
                            device_path: s.get("device_path").and_then(|v| v.as_str()).map(|x| x.to_string()),
                        }).collect()
                    }).unwrap_or_default();
                    let id = item.get("lease_id").or_else(|| item.get("id")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    composite_leases.push(CompositeLeaseTelemetry {
                        id,
                        slices,
                        priority: item.get("priority").and_then(|v| v.as_str()).unwrap_or("Interactive").to_string(),
                        state: item.get("state").and_then(|v| v.as_str()).unwrap_or("Active").to_string(),
                        client_unit: item.get("client_unit").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        client_pid: item.get("client_pid").and_then(|v| v.as_u64()).map(|u| u as u32),
                    });
                }
            }
        }

        let runtime_load = load_val.map(|lv| RuntimeLoadTelemetry {
            available_slots: lv.get("available_slots").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            max_slots: lv.get("max_slots").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            used_bytes: lv.get("used_bytes").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            models: lv.get("models").and_then(|v| v.as_array()).cloned().unwrap_or_default(),
        });

        HardwareTelemetryReport {
            gpus,
            host_ram,
            cpu,
            psi_level: format!("{:?}", psi.level),
            psi_memory_some: psi.memory_some_avg10,
            active_leases,
            composite_leases,
            runtime_load,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn offline_sockets_return_degraded_report() {
        let client = HardwareTelemetryClient::new("/tmp/no-inf.sock", "/tmp/no-rt.sock");
        let report = client.get_report().await;
        assert!(report.cpu.cores > 0);
        assert!(!report.cpu.name.is_empty());
        assert!(report.runtime_load.is_none());
    }
}
