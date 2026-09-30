use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use super::tool_types::{FunctionCall, FunctionDefinition, ToolCall, ToolDefinition};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn new(role: impl Into<String>, content: impl Into<Value>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            name: None,
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn tool_response(tool_call_id: impl Into<String>, content: impl Into<Value>) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            name: None,
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }

    pub fn with_tool_calls(mut self, calls: Vec<ToolCall>) -> Self {
        self.tool_calls = Some(calls);
        self
    }

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
        } else if self.content.is_null() {
            String::new()
        } else {
            self.content.to_string()
        }
    }

    /// Extract base64 image data from OpenAI multimodal content payload if present.
    pub fn extract_image_base64(&self) -> Option<String> {
        if let Some(arr) = self.content.as_array() {
            for item in arr {
                let item_type = item.get("type").and_then(|t| t.as_str());
                if item_type == Some("image_url") {
                    if let Some(img_url) = item.get("image_url") {
                        let url_str = if let Some(s) = img_url.as_str() {
                            Some(s)
                        } else {
                            img_url.get("url").and_then(|u| u.as_str())
                        };
                        if let Some(url) = url_str {
                            if let Some((_, b64)) = url.split_once(',') {
                                return Some(b64.to_string());
                            } else {
                                return Some(url.to_string());
                            }
                        }
                    }
                } else if item_type == Some("image") {
                    if let Some(b64) = item.get("image").or_else(|| item.get("data")).and_then(|s| s.as_str()) {
                        return Some(b64.to_string());
                    }
                }
            }
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<super::reasoning_effort::ReasoningEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, Value>,
}

impl ChatCompletionRequest {
    pub fn estimate_prompt_tokens(&self) -> usize {
        let mut chars = 0usize;
        for msg in &self.messages {
            chars += msg.role.len() + 4 + msg.content_as_str().len();
        }
        (chars as f64 / 3.8).ceil().max(1.0) as usize
    }

    pub fn estimate_total_tokens(&self) -> usize {
        self.estimate_prompt_tokens() + self.max_completion_tokens.or(self.max_tokens).unwrap_or(2048)
    }

    pub fn requested_tier(&self) -> Option<&str> {
        if let Some(t) = &self.tier { return Some(t.as_str()); }
        let m = self.model.as_str();
        if let Some(rest) = m.strip_prefix("router:") { return Some(rest); }
        if m == "fast" || m == "hard" { return Some(m); }
        None
    }

    pub fn reasoning_budget(&self) -> Option<usize> {
        self.reasoning_budget.or(self.max_thinking_tokens)
    }

    /// Extract base64 image data from messages or extra parameters if present.
    pub fn extract_image_base64(&self) -> Option<String> {
        for msg in self.messages.iter().rev() {
            if let Some(img) = msg.extract_image_base64() {
                return Some(img);
            }
        }
        if let Some(img) = self.extra.get("image").or_else(|| self.extra.get("image_base64")) {
            if let Some(s) = img.as_str() {
                if let Some((_, b64)) = s.split_once(',') {
                    return Some(b64.to_string());
                } else {
                    return Some(s.to_string());
                }
            }
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChoice {
    pub index: usize,
    pub message: ChatMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkChoice {
    pub index: usize,
    pub delta: ChunkDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
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
#[path = "chat_types_tests.rs"]
mod tests;
