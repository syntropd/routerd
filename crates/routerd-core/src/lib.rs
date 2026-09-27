//! syntrop-routerd-core
//! Core routing engine, scoring formulas, telemetry, and protocol adapters for syntrop-routerd.

pub mod adapters;
pub mod config;
pub mod credentials;
pub mod error;
pub mod models;
pub mod router;
pub mod scoring;
pub mod telemetry;

pub use config::{DaemonConfig, ProviderConfig, RouterConfig, ThresholdsConfig, TierConfig};
pub use error::{Result, RouterError};
pub use models::{
    ChatChoice, ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, ChatMessage,
    ModelItem, ModelListResponse, ProviderModelConfig, UsageInfo,
};
pub use router::{DaemonStatusInfo, ProviderStatusInfo, RouterEngine};
pub use scoring::{CandidateProvider, RequestProfile, ScoredCandidate, ScoringEngine};
pub use telemetry::{PressureLevel, PressureMetrics, TelemetryClient};

/// Default router configuration template. Lives in this crate (not the
/// repo root) so published packages stay self-contained: `cargo package`
/// only ships files under the package directory.
pub const DEFAULT_ROUTERD_TOML: &str = include_str!("../systemd/routerd.toml");
