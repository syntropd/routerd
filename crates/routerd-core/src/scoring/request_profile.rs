use crate::models::{ChatCompletionRequest, ReasoningEffort};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestProfile {
    pub requested_model: String,
    pub requested_tier: String,
    pub estimated_prompt_tokens: usize,
    pub estimated_output_tokens: usize,
    pub require_stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
}

impl RequestProfile {
    pub fn from_request(req: &ChatCompletionRequest, default_tier: &str) -> Self {
        let mut requested_tier = req
            .requested_tier()
            .map(|t| t.to_string())
            .unwrap_or_else(|| default_tier.to_string());

        let prompt_tokens = req.estimate_prompt_tokens();
        let output_tokens = req
            .max_completion_tokens
            .or(req.max_tokens)
            .unwrap_or(2048);

        // Intelligent auto-tier classification: route long or reasoning-heavy prompts to hard tier
        if requested_tier == "auto" {
            let is_complex = prompt_tokens > 1200 || req.messages.iter().any(|m| {
                let content = m.content_as_str().to_lowercase();
                content.contains("reason")
                    || content.contains("architect")
                    || content.contains("derive")
                    || content.contains("prove")
                    || content.contains("refactor")
                    || content.contains("complex")
            });
            requested_tier = if is_complex { "hard".to_string() } else { "fast".to_string() };
        }

        Self {
            requested_model: req.model.clone(),
            requested_tier,
            estimated_prompt_tokens: prompt_tokens,
            estimated_output_tokens: output_tokens,
            require_stream: req.stream.unwrap_or(false),
            reasoning_effort: req.reasoning_effort,
        }
    }

    pub fn total_tokens(&self) -> usize {
        self.estimated_prompt_tokens + self.estimated_output_tokens
    }
}
