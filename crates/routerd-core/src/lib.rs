//! syntrop-routerd-core
//! Core routing engine, scoring formulas, telemetry, and protocol adapters for syntrop-routerd.

pub mod adapters;
pub mod config;
pub mod credentials;
pub mod error;
pub mod mesh;
pub mod models;
pub mod router;
pub mod scoring;
pub mod telemetry;
pub mod wire;

pub use config::{DaemonConfig, ProviderConfig, RouterConfig, ThresholdsConfig, TierConfig};
pub use error::{Result, RouterError};
pub use mesh::{
    chunk_prefill_tokens, CircuitBreaker, CircuitState, ClusterNode, ClusterTopology,
    NodeRegistry, NodeStatus, PrefillReplayBuffer, PREFILL_CHUNK_SIZE,
};
pub use models::{
    ChatChoice, ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, ChatMessage,
    FunctionCall, FunctionDefinition, ModelItem, ModelListResponse, ProviderModelConfig, ToolCall,
    ToolDefinition, UsageInfo,
};
pub use router::{
    CascadeDecision, CascadeRouter, DaemonStatusInfo, DowngradeDecision,
    ElasticFamilyDowngrader, FamilyLadder, ProviderStatusInfo, RouterEngine, VramPressureLevel,
};
pub use scoring::{CandidateProvider, RequestProfile, ScoredCandidate, ScoringEngine};
pub use telemetry::{PressureLevel, PressureMetrics, TelemetryClient};
pub use wire::{
    configure_mesh_tcp, FilteredItem, ScmpFrame, ScmpHeader, ScmpMessageType, ThinkFilter,
    ThinkFilterState, HEADER_LEN, SCMP_MAGIC, SCMP_VERSION,
};

/// Default router configuration template. Lives in this crate (not the
/// repo root) so published packages stay self-contained: `cargo package`
/// only ships files under the package directory.
pub const DEFAULT_ROUTERD_TOML: &str = include_str!("../systemd/routerd.toml");
