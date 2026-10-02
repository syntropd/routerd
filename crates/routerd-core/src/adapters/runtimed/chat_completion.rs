use super::RuntimedAdapter;
use crate::error::Result;
use crate::models::{
    ChatChoice, ChatCompletionRequest, ChatCompletionResponse, ChatMessage, UsageInfo,
};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::debug;
use uuid::Uuid;

impl RuntimedAdapter {
    pub(super) async fn complete_chat(
        &self,
        target_model: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse> {
        let mut prompt = Self::extract_user_prompt(request);
        if let Some(tools) = request.tools.as_ref().filter(|t| !t.is_empty()) {
            if let Ok(tools_json) = serde_json::to_string(tools) {
                prompt.push_str(&format!("\nTools: {}\n", tools_json));
            }
        }
        debug!("runtimed [{}]: calling Generate for {}", self.id, target_model);
        let max_tokens = request.max_tokens.or(request.max_completion_tokens).unwrap_or(256);
        let image = request.extract_image_base64();
        let mut params = json!({
            "model": target_model,
            "prompt": prompt,
            "max_tokens": max_tokens,
            "temperature": request.temperature.unwrap_or(0.0),
            "top_k": 0,
            "top_p": request.top_p.unwrap_or(1.0),
            "seed": 0,
            "image": image,
        });
        if request.tools.as_ref().is_some_and(|t| !t.is_empty()) {
            params["grammar_type"] = json!("json");
        }
        if let Some(budget) = request.reasoning_budget() {
            params["reasoning_budget"] = json!(budget);
        }
        if let Some(effort) = request.reasoning_effort {
            params["reasoning_effort"] = json!(effort.as_str());
        }
        let psi = self.telemetry.get_pressure().await;
        let tuning = crate::telemetry::pressure_types::poll_tuning_config();
        let (k_draft, effective_max_tokens) = if psi.is_memory_pressure_spike() {
            debug!("Dynamic PSI pressure spike: shortening speculative horizon K->1 and clamping tokens");
            let _ = self.call("io.syntrop.Runtime1.CompactKvCache", json!({})).await;
            (1, max_tokens.min(tuning.max_tokens_clamp))
        } else {
            (tuning.k_draft_horizon, max_tokens)
        };

        if psi.is_cpu_contention_spike() {
            debug!("CPU scheduler contention spike ({} us); cooperative yielding within 250ms deadline", psi.runqueue_latency_us);
            tokio::task::yield_now().await;
        }

        params["max_tokens"] = json!(effective_max_tokens);

        if let Some(draft) = self.draft_models.get(target_model) {
            params["speculative_draft_model"] = json!(draft);
            params["k_draft"] = json!(k_draft);
        }
        let gen_timeout = self.compute_generate_timeout(request);
        let parameters = self
            .call_with_timeout("io.syntrop.Runtime1.Generate", params, gen_timeout)
            .await?;
        let result = parameters.get("result").cloned().unwrap_or(Value::Null);
        let text = result.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let prompt_tok = result.get("prompt_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as usize;
        let comp_tok = result.get("completion_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as usize;
        let finish = result.get("finish_reason").and_then(|f| f.as_str()).unwrap_or("stop");
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let mut filter = crate::wire::ThinkFilter::new();
        let mut items = filter.process(text);
        items.extend(filter.flush());
        let mut clean_content = String::new();
        let mut reasoning_content = String::new();
        for item in items {
            match item {
                crate::wire::FilteredItem::Content(c) => clean_content.push_str(&c),
                crate::wire::FilteredItem::Reasoning(r) => reasoning_content.push_str(&r),
            }
        }
        let final_content = if !clean_content.is_empty() {
            clean_content
        } else if finish == "length" {
            if !reasoning_content.trim().is_empty() {
                reasoning_content.clone()
            } else {
                "[Response truncated during reasoning due to token limit]".to_string()
            }
        } else if !text.is_empty() && reasoning_content.is_empty() {
            text.to_string()
        } else {
            clean_content
        };
        let final_reasoning = (!reasoning_content.is_empty()).then_some(reasoning_content);
        let mut tool_calls = None;
        let mut finish_reason = finish.to_string();
        if request.tools.as_ref().is_some_and(|t| !t.is_empty()) {
            if let Some(calls) = parse_tool_calls_from_content(&final_content) {
                tool_calls = Some(calls);
                finish_reason = "tool_calls".to_string();
            }
        }
        Ok(ChatCompletionResponse {
            id: format!("chatcmpl-runtimed-{}", Uuid::new_v4()),
            object: "chat.completion".to_string(),
            created: now,
            model: target_model.to_string(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: Value::String(final_content),
                    name: None,
                    reasoning_content: final_reasoning,
                    tool_calls: tool_calls.clone(),
                    tool_call_id: None,
                },
                tool_calls,
                finish_reason: Some(finish_reason),
            }],
            usage: Some(UsageInfo {
                prompt_tokens: prompt_tok,
                completion_tokens: comp_tok,
                total_tokens: prompt_tok + comp_tok,
            }),
        })
    }
}

/// Extracts tool calls from model output JSON if present (pure helper for testing and robustness).
pub(crate) fn parse_tool_calls_from_content(final_content: &str) -> Option<Vec<crate::models::ToolCall>> {
    let trimmed = final_content.trim();
    let unquoted = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let clean = unquoted.strip_suffix("```").unwrap_or(unquoted).trim();
    let val = serde_json::from_str::<Value>(clean).ok()?;
    let serialize_args = |raw: Option<&Value>| match raw {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => "{}".to_string(),
        Some(other) => other.to_string(),
    };
    if let Some(arr) = val.get("tool_calls").and_then(|v| v.as_array()) {
        let mut calls = Vec::new();
        for (idx, item) in arr.iter().enumerate() {
            let func_val = item.get("function");
            let name = func_val
                .and_then(|f| f.get("name"))
                .or_else(|| item.get("name"))
                .and_then(|n| n.as_str())?;
            let raw_args = func_val.and_then(|f| f.get("arguments")).or_else(|| item.get("arguments"));
            let id = item.get("id").and_then(|i| i.as_str()).map(str::to_string)
                .unwrap_or_else(|| format!("call_{}", &Uuid::new_v4().to_string()[..8]));
            let call_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("function").to_string();
            calls.push(crate::models::ToolCall {
                index: Some(idx),
                id: Some(id),
                r#type: Some(call_type),
                function: crate::models::FunctionCall {
                    name: Some(name.to_string()),
                    arguments: Some(serialize_args(raw_args)),
                },
            });
        }
        if !calls.is_empty() { return Some(calls); }
    }
    if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
        let args = serialize_args(val.get("arguments"));
        let call_id = format!("call_{}", &Uuid::new_v4().to_string()[..8]);
        return Some(vec![crate::models::ToolCall::function(call_id, name, args)]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_calls_with_string_arguments() {
        let json_str = r#"{"name": "fetch_weather", "arguments": "{\"city\":\"Honolulu\"}"}"#;
        let calls = parse_tool_calls_from_content(json_str).expect("should parse tool call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name.as_deref(), Some("fetch_weather"));
        assert_eq!(calls[0].function.arguments.as_deref(), Some("{\"city\":\"Honolulu\"}"));
        assert!(calls[0].id.as_deref().unwrap_or_default().starts_with("call_"));
    }

    #[test]
    fn parse_tool_calls_with_object_arguments() {
        let json_str = r#"{"name": "fetch_weather", "arguments": {"city":"Honolulu"}}"#;
        let calls = parse_tool_calls_from_content(json_str).expect("should parse tool call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name.as_deref(), Some("fetch_weather"));
        assert!(calls[0].function.arguments.as_deref().unwrap_or_default().contains("Honolulu"));
    }

    #[test]
    fn parse_tool_calls_with_missing_arguments() {
        let json_str = r#"{"name": "fetch_weather"}"#;
        let calls = parse_tool_calls_from_content(json_str).expect("should parse tool call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name.as_deref(), Some("fetch_weather"));
        assert_eq!(calls[0].function.arguments.as_deref(), Some("{}"));
    }

    #[test]
    fn parse_tool_calls_with_null_arguments() {
        let json_str = r#"{"name": "fetch_weather", "arguments": null}"#;
        let calls = parse_tool_calls_from_content(json_str).expect("should parse tool call");
        assert_eq!(calls[0].function.arguments.as_deref(), Some("{}"));
    }

    #[test]
    fn parse_tool_calls_standard_array() {
        let json_str = r#"{"tool_calls": [{"id": "call_abc", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]}"#;
        let calls = parse_tool_calls_from_content(json_str).expect("should parse tool call array");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id.as_deref(), Some("call_abc"));
        assert_eq!(calls[0].function.name.as_deref(), Some("lookup"));
    }

    #[test]
    fn parse_tool_calls_array_with_object_arguments() {
        let json_str = r#"{"tool_calls": [{"id": "call_abc", "type": "function", "function": {"name": "lookup", "arguments": {"city": "Honolulu"}}}]}"#;
        let calls = parse_tool_calls_from_content(json_str).expect("should parse tool call array with object args");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name.as_deref(), Some("lookup"));
        assert!(calls[0].function.arguments.as_deref().unwrap_or_default().contains("Honolulu"));
    }

    #[test]
    fn parse_tool_calls_markdown_fenced() {
        let content = "```json\n{\"name\": \"fetch_weather\", \"arguments\": {\"city\": \"Honolulu\"}}\n```";
        let calls = parse_tool_calls_from_content(content).expect("should parse fenced json");
        assert_eq!(calls[0].function.name.as_deref(), Some("fetch_weather"));
    }

    #[test]
    fn parse_tool_calls_negative_cases() {
        assert!(parse_tool_calls_from_content("not json").is_none());
        assert!(parse_tool_calls_from_content(r#"{"message": "hello"}"#).is_none());
    }
}
