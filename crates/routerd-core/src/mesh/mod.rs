//! Clustered mesh networking, node registry, and pipeline circuits.

pub mod cluster_topology;
pub mod node_registry;
pub mod pipeline_circuit;

pub use cluster_topology::ClusterTopology;
pub use node_registry::{ClusterNode, NodeRegistry, NodeStatus};
pub use pipeline_circuit::{
    chunk_prefill_tokens, CircuitBreaker, CircuitState, PrefillPipelineDispatcher,
    PrefillReplayBuffer, PREFILL_CHUNK_SIZE,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefill_chunk_size() {
        assert_eq!(PREFILL_CHUNK_SIZE, 512);
    }
}
