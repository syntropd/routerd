use std::fs;
use std::path::Path;
use std::time::Duration;

/// Timeout for a single provider probe during setup.
const PROBE_TIMEOUT_SECS: u64 = 5;
/// Files smaller than this are stubs, not models.
const MIN_REAL_MODEL_BYTES: u64 = 1_000_000;

/// Real GGUF model stems in one directory (non-recursive). Stubs and
/// sidecar files (tokenizers, partial downloads) never qualify.
pub(crate) fn scan_gguf_models(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            let is_gguf = path.extension().and_then(|x| x.to_str()) == Some("gguf");
            let is_real = entry
                .metadata()
                .map(|m| m.len() > MIN_REAL_MODEL_BYTES)
                .unwrap_or(false);
            if is_gguf && is_real {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    // Vision projectors ride along with chat models; they
                    // are not generatable models themselves.
                    if !stem.starts_with("mmproj") {
                        files.push(stem.to_string());
                    }
                }
            }
        }
    }
    files
}

/// OpenAI shape: {"data": [{"id": ...}]}.
pub fn parse_openai_model_ids(val: &serde_json::Value) -> Vec<String> {
    val.get("data")
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    m.get("id")
                        .and_then(|i| i.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// GET {base_url}/models and return the live model IDs.
/// Understands the OpenAI shape ({data:[{id}]}).
pub async fn fetch_model_ids(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .build()
        .map_err(|e| e.to_string())?;
    let base = base_url.trim_end_matches('/');
    let mut req = client.get(format!("{}/models", base));
    if let Some(k) = api_key {
        if !k.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", k.trim()));
        }
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;
    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(format!("Key rejected (HTTP {})", status));
    }
    if !status.is_success() {
        return Err(format!("Endpoint returned HTTP {}", status));
    }
    let val: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad response: {}", e))?;
    let ids = parse_openai_model_ids(&val);
    if ids.is_empty() {
        return Err("Endpoint answered but listed no models".to_string());
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

#[test]
fn test_parse_model_shapes() {
    let openai: serde_json::Value = serde_json::json!({
        "data": [{"id": "a"}, {"id": "b"}, {"nope": 1}]
    });
    assert_eq!(
        parse_openai_model_ids(&openai),
        vec!["a".to_string(), "b".to_string()]
    );
    let empty: serde_json::Value = serde_json::json!({});
    assert!(parse_openai_model_ids(&empty).is_empty());
}

#[test]
fn test_scan_gguf_models_skips_small_and_sidecars() {
    let dir = std::env::temp_dir().join(format!(
        "routerd-scan-test-{}",
        std::process::id()
    ));
    let gguf = dir.join("gguf");
    std::fs::create_dir_all(&gguf).unwrap();
    // Real file: big + .gguf. Sidecars must never qualify.
    std::fs::write(dir.join("m.gguf"), vec![0u8; 1_000_001]).unwrap();
    std::fs::write(dir.join("tiny.gguf"), vec![0u8; 10]).unwrap();
    std::fs::write(dir.join("m.tokenizer.json"), vec![0u8; 2_000_000]).unwrap();
    std::fs::write(dir.join("x.gguf.part"), vec![0u8; 2_000_000]).unwrap();
    std::fs::write(dir.join("mmproj-X-F16.gguf"), vec![0u8; 2_000_000]).unwrap();
    std::fs::write(gguf.join("n.gguf"), vec![0u8; 1_000_001]).unwrap();
    let mut found = scan_gguf_models(&dir);
    found.extend(scan_gguf_models(&gguf));
    found.sort();
    assert_eq!(found, vec!["m".to_string(), "n".to_string()]);
    std::fs::remove_dir_all(&dir).unwrap();
}
}
