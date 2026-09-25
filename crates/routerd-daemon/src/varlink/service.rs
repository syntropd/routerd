use super::protocol::VarlinkReply;
use serde_json::{json, Value};

pub const ORG_VARLINK_SERVICE_IDL: &str = r#"interface org.varlink.service

method GetInfo() -> (
  vendor: string,
  product: string,
  version: string,
  url: string,
  interfaces: []string
)

method GetInterfaceDescription(interface: string) -> (description: string)

error InterfaceNotFound(interface: string)
error MethodNotFound(method: string)
error MethodNotImplemented(method: string)
error InvalidParameter(parameter: string)
"#;

pub const IO_SYNTROP_ROUTER1_IDL: &str = r#"interface io.syntrop.Router1

type ProviderInfo (
  id: string,
  name: string,
  kind: string,
  base_url: string,
  tier: string,
  is_healthy: bool,
  weight: float,
  models: []string,
  total_requests: int,
  total_errors: int,
  last_latency_ms: float
)

type ScoredCandidateInfo (
  provider_id: string,
  model_name: string,
  total_score: float,
  speed_score: float,
  cost_score: float,
  capability_score: float,
  estimated_cost: float,
  reason: string
)

method GetStatus() -> (
  status: string,
  version: string,
  uptime_seconds: int,
  total_requests: int,
  active_requests: int,
  providers_count: int,
  healthy_providers_count: int,
  psi_level: string,
  psi_memory_some: float
)

method ListProviders() -> (
  providers: []ProviderInfo
)

method ListModels() -> (
  models: []string
)

method RouteRequest(
  model: ?string,
  tier: ?string,
  estimated_tokens: ?int,
  require_stream: ?bool
) -> (
  candidates: []ScoredCandidateInfo
)

method TestProvider(
  provider_id: string
) -> (
  provider_id: string,
  healthy: bool,
  latency_ms: float,
  error: ?string
)
"#;

pub fn handle_get_info() -> VarlinkReply {
    VarlinkReply::ok(json!({
        "vendor": "syntropd",
        "product": "routerd",
        "version": "0.3.0",
        "url": "https://github.com/syntropd/routerd",
        "interfaces": [
            "org.varlink.service",
            "io.syntrop.Router1"
        ]
    }))
}

pub fn handle_get_interface_description(params: Option<&Value>) -> VarlinkReply {
    let interface_name = params
        .and_then(|p| p.get("interface"))
        .and_then(|v| v.as_str());

    match interface_name {
        Some("org.varlink.service") => VarlinkReply::ok(json!({
            "description": ORG_VARLINK_SERVICE_IDL
        })),
        Some("io.syntrop.Router1") => VarlinkReply::ok(json!({
            "description": IO_SYNTROP_ROUTER1_IDL
        })),
        Some(unknown) => VarlinkReply::error(
            "org.varlink.service.InterfaceNotFound",
            json!({ "interface": unknown }),
        ),
        None => VarlinkReply::error(
            "org.varlink.service.InvalidParameter",
            json!({ "parameter": "interface" }),
        ),
    }
}
