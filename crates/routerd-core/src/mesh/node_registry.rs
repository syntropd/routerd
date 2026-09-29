//! Cluster node registry tracking active cluster members and their capabilities.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Active,
    Draining,
    Disconnected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNode {
    pub id: String,
    pub address: String,
    pub status: NodeStatus,
    pub total_vram_bytes: u64,
    pub available_vram_bytes: u64,
    pub latency_ms: u32,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct NodeRegistry {
    nodes: HashMap<String, ClusterNode>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self { nodes: HashMap::new() }
    }

    pub fn register_node(&mut self, node: ClusterNode) {
        self.nodes.insert(node.id.clone(), node);
    }

    pub fn get_node(&self, id: &str) -> Option<&ClusterNode> {
        self.nodes.get(id)
    }

    pub fn update_heartbeat(&mut self, id: &str, latency_ms: u32) -> bool {
        if let Some(node) = self.nodes.get_mut(id) {
            node.last_seen = Utc::now();
            node.latency_ms = latency_ms;
            node.status = NodeStatus::Active;
            true
        } else {
            false
        }
    }

    pub fn mark_disconnected(&mut self, id: &str) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.status = NodeStatus::Disconnected;
        }
    }

    pub fn active_nodes(&self) -> Vec<ClusterNode> {
        self.nodes
            .values()
            .filter(|n| n.status == NodeStatus::Active)
            .cloned()
            .collect()
    }

    pub fn prune_stale_nodes(&mut self, timeout_secs: i64) -> usize {
        let now = Utc::now();
        let mut pruned = 0;
        for node in self.nodes.values_mut() {
            if node.status == NodeStatus::Active
                && (now - node.last_seen).num_seconds() > timeout_secs
            {
                node.status = NodeStatus::Disconnected;
                pruned += 1;
            }
        }
        pruned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_lifecycle_and_pruning() {
        let mut reg = NodeRegistry::new();
        let node = ClusterNode {
            id: "node-1".into(),
            address: "10.0.0.1:9099".into(),
            status: NodeStatus::Active,
            total_vram_bytes: 16 << 30,
            available_vram_bytes: 16 << 30,
            latency_ms: 2,
            last_seen: Utc::now(),
        };
        reg.register_node(node);
        assert_eq!(reg.active_nodes().len(), 1);

        reg.mark_disconnected("node-1");
        assert_eq!(reg.active_nodes().len(), 0);
    }
}
