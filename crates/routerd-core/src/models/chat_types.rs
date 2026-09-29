use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn content_as_str(&self) -> String {
        if let Some(s) = self.content.as_str() {
            s.to_string()
        } else if let Some(arr) = self.content.as_array() {
            let mut out = String::new();
            for item in arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    out.push_str(text);
                    out.push(' ');
                }
            }
            out
        } else {
            self.content.to_string()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_budget: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_thinking_tokens: Option<usize>,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, Value>,
}

impl ChatCompletionRequest {
    /// Estimate prompt tokens based on characters (heuristic: ~3.8 chars per token + message framing)
    pub fn estimate_prompt_tokens(&self) -> usize {
        let mut chars = 0usize;
        for msg in &self.messages {
            chars += msg.role.len() + 4;
            chars += msg.content_as_str().len();
        }
        let token_estimate = (chars as f64 / 3.8).ceil() as usize;
        token_estimate.max(1)
    }

    /// Estimate total context length (prompt tokens + expected output tokens)
    pub fn estimate_total_tokens(&self) -> usize {
        let prompt_tokens = self.estimate_prompt_tokens();
        let completion_tokens = self
            .max_completion_tokens
            .or(self.max_tokens)
            .unwrap_or(2048);
        prompt_tokens + completion_tokens
    }

    /// Extract difficulty tier requested (e.g. from model name like `router:fast` or tier field)
    pub fn requested_tier(&self) -> Option<&str> {
        if let Some(t) = &self.tier {
            return Some(t.as_str());
        }
        let m = self.model.as_str();
        if m.starts_with("router:") {
            return Some(&m["router:".len()..]);
        }
        if m == "fast" || m == "hard" {
            return Some(m);
        }
        None
    }

    /// Extract effective reasoning token budget if requested.
    pub fn reasoning_budget(&self) -> Option<usize> {
        self.reasoning_budget.or(self.max_thinking_tokens)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChoice {
    pub index: usize,
    pub message: ChatMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageInfo {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkChoice {
    pub index: usize,
    pub delta: ChunkDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChunkChoice>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn request(model: &str, tier: Option<&str>, max_tokens: Option<usize>) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: model.to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: serde_json::json!("hello"),
                name: None,
            }],
            temperature: None,
            top_p: None,
            max_tokens,
            max_completion_tokens: None,
            stream: None,
            tier: tier.map(str::to_string),
            reasoning_budget: None,
            max_thinking_tokens: None,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn content_as_str_reads_all_shapes() {
        let plain = ChatMessage {
            role: "r".to_string(),
            content: serde_json::json!("hi"),
            name: None,
        };
        assert_eq!(plain.content_as_str(), "hi");
        let parts = ChatMessage {
            role: "r".to_string(),
            content: serde_json::json!([{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]),
            name: None,
        };
        assert_eq!(parts.content_as_str(), "a b ");
    }

    #[test]
    fn requested_tier_prefers_field_then_alias() {
        assert_eq!(request("x", Some("hard"), None).requested_tier(), Some("hard"));
        assert_eq!(request("router:fast", None, None).requested_tier(), Some("fast"));
        assert_eq!(request("hard", None, None).requested_tier(), Some("hard"));
        assert_eq!(request("llama", None, None).requested_tier(), None);
    }

    #[test]
    fn token_estimates_floor_at_one_and_add_output() {
        let mut req = request("router:fast", None, Some(100));
        req.messages.clear();
        assert_eq!(req.estimate_prompt_tokens(), 1);
        assert_eq!(req.estimate_total_tokens(), 101);
        let req = request("router:fast", None, None);
        assert!(req.estimate_prompt_tokens() >= 1);
        assert_eq!(req.estimate_total_tokens(), req.estimate_prompt_tokens() + 2048);
    }

    #[test]
    fn reasoning_budget_prefers_explicit_then_max_thinking() {
        let mut req = request("test", None, None);
        assert_eq!(req.reasoning_budget(), None);
        req.max_thinking_tokens = Some(500);
        assert_eq!(req.reasoning_budget(), Some(500));
        req.reasoning_budget = Some(1000);
        assert_eq!(req.reasoning_budget(), Some(1000));
    }
}
