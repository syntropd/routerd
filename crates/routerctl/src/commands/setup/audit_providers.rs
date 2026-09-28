use super::edit_config::set_models_array;
use super::probe_models::{fetch_model_ids, scan_gguf_models};
use super::register::register_provider::RUNTIMED_SOCKET;
use super::register::{clear_provider_models, upsert_runtimed_models};
use colored::Colorize;
use futures::future::join_all;
use routerd_core::config::is_local_provider;
use routerd_core::credentials::resolve_credential;
use std::path::Path;
use toml_edit::{DocumentMut, Item, Value};

struct AuditEntry {
    idx: usize,
    id: String,
    kind: String,
    base_url: String,
    key_spec: Option<String>,
}

/// Provider kinds whose base URL is a local socket path, not HTTP.
/// These get a socket-existence probe instead of an HTTP ping.
fn is_socket_kind(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "varlink" | "syntrop" | "runtimed"
    )
}

/// Ping every enabled provider already in the document, in parallel.
/// Dead entries get switched off; live ones get their model lists refreshed
/// from the answers. Never prompts.
pub(crate) async fn audit_existing_providers(doc: &mut DocumentMut, report: &mut Vec<(String, String)>) {
    let mut entries: Vec<AuditEntry> = Vec::new();
    if let Some(arr) = doc.get("providers").and_then(|p| p.as_array_of_tables()) {
        for (idx, table) in arr.iter().enumerate() {
            if table.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                continue;
            }
            let id = table
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            let kind = table
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("openai")
                .to_string();
            let base_url = table
                .get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let key_spec = table
                .get("api_key")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            entries.push(AuditEntry {
                idx,
                id,
                kind,
                base_url,
                key_spec,
            });
        }
    }
    if entries.is_empty() {
        println!("  nothing configured yet");
        return;
    }
    // Local-only mode: external entries are switched off and their keys
    // stripped without ever being pinged. The entries stay in the file so
    // restoring cloud later is just re-enabling them.
    let (cloud, local): (Vec<AuditEntry>, Vec<AuditEntry>) = entries
        .into_iter()
        .partition(|e| !is_local_provider(&e.kind, &e.base_url));
    for e in &cloud {
        if let Some(table) = doc
            .get_mut("providers")
            .and_then(|p| p.as_array_of_tables_mut())
            .and_then(|a| a.get_mut(e.idx))
        {
            table.insert("enabled", Item::Value(Value::from(false)));
            table.remove("api_key");
        }
        println!("  {:<16} OFF · external (local-only mode)", e.id.bold());
        report.push((e.id.clone(), "OFF · external (local-only mode)".to_string()));
    }
    let probes = local.iter().map(|e| async move {
        if is_socket_kind(&e.kind) {
            if Path::new(&e.base_url).exists() {
                (e.idx, e.id.clone(), Ok(Vec::new()))
            } else {
                (
                    e.idx,
                    e.id.clone(),
                    Err("local socket missing".to_string()),
                )
            }
        } else {
            let key = e
                .key_spec
                .as_deref()
                .and_then(|spec| resolve_credential(Some(spec), &e.id).unwrap_or(None));
            match fetch_model_ids(&e.base_url, key.as_deref()).await {
                Ok(ids) => (e.idx, e.id.clone(), Ok(ids)),
                Err(err) => (e.idx, e.id.clone(), Err(err)),
            }
        }
    });
    for (idx, id, result) in join_all(probes).await {
        let arr = match doc
            .get_mut("providers")
            .and_then(|p| p.as_array_of_tables_mut())
        {
            Some(a) => a,
            None => continue,
        };
        let Some(table) = arr.get_mut(idx) else {
            continue;
        };
        match result {
            Ok(ids) => {
                if !ids.is_empty() {
                    set_models_array(table, &ids);
                }
                table.insert("enabled", Item::Value(Value::from(true)));
                let line = if ids.is_empty() {
                    "LIVE · local".to_string()
                } else {
                    format!("LIVE · {} model(s)", ids.len())
                };
                println!("  {:<16} {}", id.bold(), line);
                report.push((id, line));
            }
            Err(e) => {
                table.insert("enabled", Item::Value(Value::from(false)));
                println!("  {:<16} {} {}", id.bold(), "OFF ·".red().bold(), e);
                report.push((id, format!("OFF · {}", e)));
            }
        }
    }
}

/// Find real on-disk model files and register them under the owned
/// engine. Registers only what exists. Never prompts.
pub(crate) async fn autodetect_local(
    doc: &mut DocumentMut,
    models_dir: &Path,
    report: &mut Vec<(String, String)>,
) {
    let mut found_any = false;
    let mut files = scan_gguf_models(models_dir);
    files.extend(scan_gguf_models(&models_dir.join("gguf")));
    files.sort();
    files.dedup();
    if files.is_empty() {
        println!("  {:<16} none in {}", "model files".bold(), models_dir.display());
    } else {
        let alive = Path::new(RUNTIMED_SOCKET).exists();
        upsert_runtimed_models(doc, &files, alive);
        // GGUF files belong to the real engine. The legacy broker entry
        // cannot serve them, so release any names it still claims.
        clear_provider_models(doc, "syntrop-local");
        if alive {
            println!("  {:<16} LIVE · {} file(s)", "model files".bold(), files.len());
            report.push((
                "runtimed-local".to_string(),
                format!("LIVE · {} file(s)", files.len()),
            ));
        } else {
            println!("  {:<16} OFF · engine not running", "model files".bold());
            report.push((
                "runtimed-local".to_string(),
                "OFF · engine not running".to_string(),
            ));
        }
        found_any = true;
    }
    if !found_any {
        println!("  hint: place GGUF files under /var/lib/models/gguf, re-run setup");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

#[test]
fn test_socket_kinds_skip_http_ping() {
    assert!(is_socket_kind("runtimed"));
    assert!(is_socket_kind("varlink"));
    assert!(is_socket_kind("syntrop"));
    assert!(is_socket_kind("Runtimed"));
    assert!(!is_socket_kind("openai"));
    assert!(!is_socket_kind("minimax"));
    assert!(!is_socket_kind("ollama"));
}
}
