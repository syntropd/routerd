//! Cascaded routing ("System 1 -> System 2") routing simple queries to CPU.

use crate::models::{ChatCompletionRequest, ReasoningEffort};

/// Classification decision for cascaded routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CascadeDecision {
    /// System 1: Fast, reflexive CPU path (<20ms SLA) for simple queries.
    System1Cpu {
        model: String,
        target_latency_ms: u64,
    },
    /// System 2: Deep, deliberate GPU path for complex reasoning, code, or tools.
    System2Gpu {
        model: String,
        reason: String,
    },
}

/// Router classifying queries into System 1 (fast CPU) or System 2 (deep GPU).
#[derive(Debug, Clone)]
pub struct CascadeRouter {
    cpu_model: String,
    gpu_model: String,
    max_system1_prompt_tokens: usize,
    target_cpu_latency_ms: u64,
}

impl Default for CascadeRouter {
    fn default() -> Self {
        Self::new("qwen2.5-0.5b", "qwen2.5-7b")
    }
}

impl CascadeRouter {
    /// Create a new CascadeRouter with designated CPU draft and GPU target model names.
    pub fn new(cpu_model: impl Into<String>, gpu_model: impl Into<String>) -> Self {
        Self {
            cpu_model: cpu_model.into(),
            gpu_model: gpu_model.into(),
            max_system1_prompt_tokens: 64,
            target_cpu_latency_ms: 20,
        }
    }

    /// Classify a chat completion request into System 1 (CPU) or System 2 (GPU).
    pub fn classify(&self, req: &ChatCompletionRequest) -> CascadeDecision {
        // Rule 1: Tool calling requires deep System 2 GPU processing
        if let Some(ref tools) = req.tools {
            if !tools.is_empty() {
                return CascadeDecision::System2Gpu {
                    model: self.gpu_model.clone(),
                    reason: "request specifies tools".into(),
                };
            }
        }

        // Rule 2: Explicit high/max reasoning effort escalates to System 2
        if let Some(effort) = req.reasoning_effort {
            if matches!(effort, ReasoningEffort::High | ReasoningEffort::Max) {
                return CascadeDecision::System2Gpu {
                    model: self.gpu_model.clone(),
                    reason: "high reasoning effort requested".into(),
                };
            }
        }

        // Rule 3: Token budget / prompt length threshold
        let prompt_tokens = req.estimate_prompt_tokens();
        if prompt_tokens > self.max_system1_prompt_tokens {
            return CascadeDecision::System2Gpu {
                model: self.gpu_model.clone(),
                reason: format!("prompt tokens ({prompt_tokens}) exceed System 1 ceiling ({})", self.max_system1_prompt_tokens),
            };
        }

        // Rule 4: Structural query complexity heuristics
        for msg in &req.messages {
            let text = msg.content_as_str();
            if self.is_complex_text(&text) {
                return CascadeDecision::System2Gpu {
                    model: self.gpu_model.clone(),
                    reason: "complex syntax or keyword patterns detected".into(),
                };
            }
        }

        // Simple query resolves directly on CPU within target latency (<20ms)
        CascadeDecision::System1Cpu {
            model: self.cpu_model.clone(),
            target_latency_ms: self.target_cpu_latency_ms,
        }
    }

    /// Heuristic detecting code snippets, derivation requests, or algorithmic queries.
    fn is_complex_text(&self, s: &str) -> bool {
        if s.contains("```") || s.contains("fn ") || s.contains("def ") || s.contains("class ") {
            return true;
        }
        let lower = s.to_ascii_lowercase();
        let triggers = [
            "write code",
            "debug",
            "prove that",
            "derive",
            "implement",
            "analyze architecture",
            "explain in detail",
        ];
        triggers.iter().any(|t| lower.contains(t))
    }

    /// Apply cascade decision to request, rewriting model if routing to System 1 CPU.
    pub fn apply(&self, req: &mut ChatCompletionRequest) -> CascadeDecision {
        let decision = self.classify(req);
        if let CascadeDecision::System1Cpu { ref model, .. } = decision {
            req.model = model.clone();
        }
        decision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ChatMessage;

    #[test]
    fn test_simple_greeting_routes_to_system1_cpu() {
        let router = CascadeRouter::default();
        let req = ChatCompletionRequest {
            messages: vec![ChatMessage::new("user", "Hello, how are you?")],
            ..Default::default()
        };
        let decision = router.classify(&req);
        assert!(matches!(decision, CascadeDecision::System1Cpu { target_latency_ms: 20, .. }));
    }

    #[test]
    fn test_code_snippet_routes_to_system2_gpu() {
        let router = CascadeRouter::default();
        let req = ChatCompletionRequest {
            messages: vec![ChatMessage::new("user", "```rust\nfn main() {}\n```")],
            ..Default::default()
        };
        let decision = router.classify(&req);
        assert!(matches!(decision, CascadeDecision::System2Gpu { .. }));
    }

    #[test]
    fn test_high_reasoning_routes_to_system2_gpu() {
        let router = CascadeRouter::default();
        let req = ChatCompletionRequest {
            messages: vec![ChatMessage::new("user", "short question")],
            reasoning_effort: Some(ReasoningEffort::High),
            ..Default::default()
        };
        let decision = router.classify(&req);
        assert!(matches!(decision, CascadeDecision::System2Gpu { .. }));
    }
}
