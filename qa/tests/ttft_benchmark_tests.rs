use serde_json::Value;
use std::time::{Duration, Instant};

#[test]
fn test_ttft_calculation_from_sse_stream() {
    let start = Instant::now();

    // Simulate SSE chunks
    let chunks = vec![
        "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"}}]}\n\n",
        "data: [DONE]\n\n",
    ];

    let mut first_token_instant = None;
    let mut accumulated = String::new();
    let mut chunk_count = 0;

    std::thread::sleep(Duration::from_millis(5));

    for raw in chunks {
        for line in raw.lines() {
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
                        .and_then(|t| t.as_str())
                    {
                        if first_token_instant.is_none() && !delta.is_empty() {
                            first_token_instant = Some(Instant::now());
                        }
                        accumulated.push_str(delta);
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

    assert!(ttft_ms > 0.0);
    assert!(total_latency_ms >= ttft_ms);
    assert_eq!(accumulated, "Hello world");
    assert_eq!(chunk_count, 2);
}
