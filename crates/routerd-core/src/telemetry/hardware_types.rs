use serde::{Deserialize, Serialize};

/// Point-to-point interconnect telemetry between compute planes.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DeviceLinkTelemetry {
    pub peer_plane_id: String,
    pub link_type: String,
    pub bandwidth_bytes_sec: u64,
    pub latency_nanos: u64,
}

/// DRM GPU VRAM allocation and feature telemetry from inferenced.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DrmGpuTelemetry {
    pub id: String,
    pub name: String,
    pub total_vram_bytes: u64,
    pub used_vram_bytes: u64,
    pub available_vram_bytes: u64,
    pub features: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p2p_links: Option<Vec<DeviceLinkTelemetry>>,
    #[serde(default)]
    pub kernel_used_memory: u64,
}

/// Host system RAM utilization telemetry.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HostRamTelemetry {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
}

/// Host CPU SIMD/Matrix accelerator capabilities and cores.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CpuMatrixTelemetry {
    pub name: String,
    pub cores: usize,
    pub features: Vec<String>,
}

/// Active compute lease metadata allocated by inferenced arbiter.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ComputeLeaseTelemetry {
    pub id: String,
    pub plane_id: String,
    pub allocated_memory: u64,
    pub priority: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_pid: Option<u32>,
}

/// Individual slice allocation in a composite lease.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CompositeSliceTelemetry {
    pub plane_id: String,
    pub allocated_memory: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_path: Option<String>,
}

/// Active composite compute lease across multiple devices from inferenced.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CompositeLeaseTelemetry {
    pub id: String,
    pub slices: Vec<CompositeSliceTelemetry>,
    pub priority: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_pid: Option<u32>,
}

/// Concurrent execution load and memory from runtimed.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct RuntimeLoadTelemetry {
    pub available_slots: usize,
    pub max_slots: usize,
    pub used_bytes: usize,
    pub models: Vec<serde_json::Value>,
}

/// Consolidated hardware, lease, and runtime telemetry report.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HardwareTelemetryReport {
    pub gpus: Vec<DrmGpuTelemetry>,
    pub host_ram: HostRamTelemetry,
    pub cpu: CpuMatrixTelemetry,
    pub psi_level: String,
    pub psi_memory_some: f32,
    pub active_leases: Vec<ComputeLeaseTelemetry>,
    #[serde(default)]
    pub composite_leases: Vec<CompositeLeaseTelemetry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_load: Option<RuntimeLoadTelemetry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hardware_telemetry_roundtrip() {
        let report = HardwareTelemetryReport {
            gpus: vec![DrmGpuTelemetry {
                id: "renderD128".to_string(),
                name: "Intel Arc".to_string(),
                total_vram_bytes: 16 * 1024 * 1024 * 1024,
                used_vram_bytes: 4 * 1024 * 1024 * 1024,
                available_vram_bytes: 12 * 1024 * 1024 * 1024,
                features: vec!["vulkan".to_string(), "oneapi".to_string()],
                p2p_links: None,
                kernel_used_memory: 0,
            }],
            host_ram: HostRamTelemetry {
                total_bytes: 32 * 1024 * 1024 * 1024,
                available_bytes: 24 * 1024 * 1024 * 1024,
                used_bytes: 8 * 1024 * 1024 * 1024,
            },
            cpu: CpuMatrixTelemetry {
                name: "Intel Xeon".to_string(),
                cores: 16,
                features: vec!["avx512".to_string(), "amx".to_string()],
            },
            psi_level: "Normal".to_string(),
            psi_memory_some: 0.0,
            active_leases: vec![ComputeLeaseTelemetry {
                id: "lease-1".to_string(),
                plane_id: "renderD128".to_string(),
                allocated_memory: 1024,
                priority: "Interactive".to_string(),
                state: "Active".to_string(),
                client_unit: None,
                client_pid: None,
            }],
            composite_leases: vec![CompositeLeaseTelemetry {
                id: "comp-1".to_string(),
                slices: vec![CompositeSliceTelemetry {
                    plane_id: "renderD128".to_string(),
                    allocated_memory: 1024,
                    device_path: Some("/dev/dri/renderD128".to_string()),
                }],
                priority: "Interactive".to_string(),
                state: "Active".to_string(),
                client_unit: None,
                client_pid: None,
            }],
            runtime_load: Some(RuntimeLoadTelemetry {
                available_slots: 4,
                max_slots: 4,
                used_bytes: 0,
                models: vec![],
            }),
        };

        let json_str = serde_json::to_string(&report).unwrap();
        let decoded: HardwareTelemetryReport = serde_json::from_str(&json_str).unwrap();
        assert_eq!(report, decoded);
    }
}
