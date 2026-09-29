//! Chunked prefill pipelining, circuit breakers, and atomic prefill replay.

pub const PREFILL_CHUNK_SIZE: usize = 1024;

/// Partition a sequence of prompt tokens into 1024-token micro-batches.
pub fn chunk_prefill_tokens(tokens: &[u32]) -> Vec<Vec<u32>> {
    tokens
        .chunks(PREFILL_CHUNK_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

pub struct CircuitBreaker {
    pub failure_count: usize,
    pub threshold: usize,
    pub state: CircuitState,
}

impl CircuitBreaker {
    pub fn new(threshold: usize) -> Self {
        Self {
            failure_count: 0,
            threshold,
            state: CircuitState::Closed,
        }
    }

    pub fn record_success(&mut self) {
        self.failure_count = 0;
        self.state = CircuitState::Closed;
    }

    pub fn record_failure(&mut self) -> CircuitState {
        self.failure_count += 1;
        if self.failure_count >= self.threshold {
            self.state = CircuitState::Open;
        }
        self.state
    }

    pub fn can_attempt(&self) -> bool {
        self.state != CircuitState::Open
    }

    pub fn transition_to_half_open(&mut self) {
        self.state = CircuitState::HalfOpen;
    }
}

/// Buffer tracking unacknowledged in-flight prefill chunks for atomic replay.
#[derive(Debug, Clone)]
pub struct PrefillReplayBuffer {
    pub in_flight: Vec<(u64, Vec<u32>)>,
    pub target_node: String,
}

impl PrefillReplayBuffer {
    pub fn new(target_node: String) -> Self {
        Self {
            in_flight: Vec::new(),
            target_node,
        }
    }

    pub fn track_chunk(&mut self, seq: u64, tokens: Vec<u32>) {
        self.in_flight.push((seq, tokens));
    }

    pub fn ack_chunk(&mut self, seq: u64) {
        self.in_flight.retain(|(s, _)| *s != seq);
    }

    /// On node failure, drain all pending in-flight chunks to replay atomically to replacement node.
    pub fn drain_for_atomic_replay(&mut self, replacement_node: String) -> (String, Vec<(u64, Vec<u32>)>) {
        let old_node = std::mem::replace(&mut self.target_node, replacement_node);
        let chunks = std::mem::take(&mut self.in_flight);
        (old_node, chunks)
    }

    /// Re-arm pending chunks if a fallback node also fails before ack (cascading failure resilience).
    pub fn rearm_chunks(&mut self, chunks: Vec<(u64, Vec<u32>)>) {
        let mut combined = chunks;
        combined.extend(std::mem::take(&mut self.in_flight));
        self.in_flight = combined;
    }

    pub fn pending_count(&self) -> usize {
        self.in_flight.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunked_prefill_1024_microbatches() {
        let tokens: Vec<u32> = (0..2500).collect();
        let chunks = chunk_prefill_tokens(&tokens);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 1024);
        assert_eq!(chunks[1].len(), 1024);
        assert_eq!(chunks[2].len(), 452);
    }

    #[test]
    fn test_circuit_breaker_and_atomic_replay() {
        let mut cb = CircuitBreaker::new(2);
        assert!(cb.can_attempt());
        assert_eq!(cb.record_failure(), CircuitState::Closed);
        assert_eq!(cb.record_failure(), CircuitState::Open);
        assert!(!cb.can_attempt());

        cb.transition_to_half_open();
        assert_eq!(cb.state, CircuitState::HalfOpen);
        assert!(cb.can_attempt());

        let mut replay = PrefillReplayBuffer::new("node-alpha".into());
        replay.track_chunk(1, vec![10, 20]);
        replay.track_chunk(2, vec![30, 40]);
        replay.ack_chunk(1);
        assert_eq!(replay.pending_count(), 1);

        let (failed_node, pending) = replay.drain_for_atomic_replay("node-beta".into());
        assert_eq!(failed_node, "node-alpha");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, 2);
        assert_eq!(replay.target_node, "node-beta");

        // Cascading failure: node-beta fails immediately, rearm chunks for node-gamma
        replay.rearm_chunks(pending);
        assert_eq!(replay.pending_count(), 1);
        let (failed_second, pending_second) = replay.drain_for_atomic_replay("node-gamma".into());
        assert_eq!(failed_second, "node-beta");
        assert_eq!(pending_second.len(), 1);
        assert_eq!(pending_second[0].0, 2);
        assert_eq!(replay.target_node, "node-gamma");
    }
}
