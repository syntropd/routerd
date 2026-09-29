use super::RouterctlClient;
use anyhow::{anyhow, Result};
use futures::StreamExt;
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub model: String,
    pub ttft_ms: f64,
    pub total_latency_ms: f64,
    pub chunks: usize,
    pub content: String,
}

impl RouterctlClient {
    pub async fn test_benchmark(
        &self,
        model: &str,
        prompt: &str,
        tier: Option<&str>,
    ) -> Result<BenchmarkResult> {
        let start = Instant::now();
        let url = format!("{}/v1/chat/completions", self.http_url);

        let mut body = json!({
            "model": model,
            "messages": [
                { "role": "user", "content": prompt }
            ],
            "max_tokens": 64,
            "stream": true
        });
        if let Some(t) = tier {
            body["tier"] = json!(t);
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

        let mut stream = resp.bytes_stream();
        let mut first_token_instant = None;
        let mut accumulated_content = String::new();
        let mut chunk_count = 0usize;

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res?;
            let text = String::from_utf8_lossy(&chunk);
            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    let trimmed = data.trim();
                    if trimmed == "[DONE]" {
                        continue;
                    }
                    if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
                        if let Some(delta) = val
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c0| c0.get("delta"))
                            .and_then(|d| d.get("content"))
                            .and_then(|txt| txt.as_str())
                        {
                            if first_token_instant.is_none() && !delta.is_empty() {
                                first_token_instant = Some(Instant::now());
                            }
                            accumulated_content.push_str(delta);
                            chunk_count += 1;
                        }
                    }
                }
            }
        }

        let total_latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        let ttft_ms = first_token_instant
            .map(|t| t.duration_since(start).as_secs_f64() * 1000.0)
            .unwrap_or(total_latency_ms);

        Ok(BenchmarkResult {
            model: model.to_string(),
            ttft_ms,
            total_latency_ms,
            chunks: chunk_count,
            content: accumulated_content,
        })
    }
}
