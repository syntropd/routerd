use super::RouterctlClient;
use anyhow::{anyhow, Result};
use reqwest::Client as HttpClient;
use serde_json::{json, Value};
use std::time::Duration;

impl RouterctlClient {
    /// Non-streaming completion: send one prompt, return the reply text.
    /// Local CPU answers can take minutes, so this call gets its own
    /// generous timeout instead of the snappy default client.
    pub async fn chat_completion(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: usize,
        effort: Option<&str>,
    ) -> Result<String> {
        let url = format!("{}/v1/chat/completions", self.http_url);
        let mut body = json!({
            "model": model,
            "messages": [
                { "role": "user", "content": prompt }
            ],
            "max_tokens": max_tokens,
            "stream": false
        });
        if let Some(e) = effort {
            body["reasoning_effort"] = json!(e);
        }
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .unwrap_or_default();
        let resp = http.post(&url).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let err_txt = resp.text().await.unwrap_or_default();
            return Err(anyhow!("HTTP {}: {}", status, err_txt));
        }
        let val: Value = resp.json().await?;
        let text = val
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c0| c0.get("message"))
            .map(message_text)
            .unwrap_or_default();
        if text.is_empty() {
            return Err(anyhow!("router returned an empty reply"));
        }
        Ok(text)
    }
}

/// Reply text from a chat message object. Content arrives as a plain
/// string from local providers or as content-parts from cloud ones.
fn message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_text_reads_string_and_parts() {
        assert_eq!(
            message_text(&json!({ "role": "assistant", "content": "hi" })),
            "hi"
        );
        assert_eq!(
            message_text(&json!({ "content": [{ "type": "text", "text": "a" }, { "type": "text", "text": "b" }] })),
            "ab"
        );
        assert!(message_text(&json!({})).is_empty());
    }
}
