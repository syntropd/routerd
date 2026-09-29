//! Cluster topology and multi-node pipeline stage routing.

use super::node_registry::NodeRegistry;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ClusterTopology {
    stage_assignments: HashMap<usize, String>,
    node_hops: HashMap<(String, String), u32>,
}

impl ClusterTopology {
    pub fn new() -> Self {
        Self {
            stage_assignments: HashMap::new(),
            node_hops: HashMap::new(),
        }
    }

    pub fn assign_stage(&mut self, stage: usize, node_id: String) {
        self.stage_assignments.insert(stage, node_id);
    }

    pub fn get_stage_node(&self, stage: usize) -> Option<&str> {
        self.stage_assignments.get(&stage).map(|s| s.as_str())
    }

    pub fn set_hop_distance(&mut self, node_a: &str, node_b: &str, hops: u32) {
        self.node_hops.insert((node_a.to_string(), node_b.to_string()), hops);
        self.node_hops.insert((node_b.to_string(), node_a.to_string()), hops);
    }

    pub fn get_hop_distance(&self, node_a: &str, node_b: &str) -> u32 {
        if node_a == node_b {
            return 0;
        }
        self.node_hops
            .get(&(node_a.to_string(), node_b.to_string()))
            .copied()
            .unwrap_or(2) // Default inter-node hop distance
    }

    /// Select an alternate active node with lowest hop distance when a node fails.
    pub fn find_fallback_node(&self, registry: &NodeRegistry, failed_node: &str) -> Option<String> {
        let active = registry.active_nodes();
        active
            .into_iter()
            .filter(|n| n.id != failed_node)
            .min_by_key(|n| self.get_hop_distance(failed_node, &n.id))
            .map(|n| n.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::node_registry::ClusterNode;
    use chrono::Utc;

    #[test]
    fn test_stage_routing_and_fallback() {
        let mut topo = ClusterTopology::new();
        topo.assign_stage(0, "node-a".into());
        topo.assign_stage(1, "node-b".into());
        topo.set_hop_distance("node-a", "node-b", 1);
        topo.set_hop_distance("node-a", "node-c", 3);

        let mut reg = NodeRegistry::new();
        reg.register_node(ClusterNode {
            id: "node-b".into(),
            address: "10.0.0.2:9099".into(),
            status: crate::mesh::node_registry::NodeStatus::Active,
            total_vram_bytes: 16 << 30,
            available_vram_bytes: 16 << 30,
            latency_ms: 1,
            last_seen: Utc::now(),
        });

        let fallback = topo.find_fallback_node(&reg, "node-a").unwrap();
        assert_eq!(fallback, "node-b");
    }
}
