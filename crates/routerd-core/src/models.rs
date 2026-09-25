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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelItem {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub owned_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelListResponse {
    pub object: String,
    pub data: Vec<ModelItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelConfig {
    pub name: String,
    #[serde(default = "default_max_context")]
    pub max_context_tokens: usize,
    #[serde(default)]
    pub cost_per_input_token: f64,
    #[serde(default)]
    pub cost_per_output_token: f64,
    #[serde(default = "default_latency")]
    pub avg_latency_ms: f64,
    #[serde(default = "default_tps")]
    pub tokens_per_second: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
}

fn default_max_context() -> usize {
    32768
}

fn default_latency() -> f64 {
    200.0
}

fn default_tps() -> f64 {
    50.0
}
