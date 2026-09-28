use crate::client::RouterctlClient;
use colored::Colorize;
use std::path::PathBuf;
use std::time::Duration;
use toml_edit::{DocumentMut, Table};

/// Engine warmup budget: a cold multi-GB load plus one token.
const WARMUP_TIMEOUT: Duration = Duration::from_secs(600);

/// First model name from a provider table (string list or name tables).
fn first_model_name(table: &Table) -> Option<String> {
    let models = table.get("models")?;
    if let Some(arr) = models.as_array() {
        return arr.iter().filter_map(|v| v.as_str()).map(str::to_string).next();
    }
    if let Some(tables) = models.as_array_of_tables() {
        return tables
            .iter()
            .filter_map(|t| t.get("name")?.as_str())
            .map(str::to_string)
            .next();
    }
    None
}

/// The owned engine's (socket, model) when one is registered, enabled,
/// and present on disk.
fn warmup_target(doc: &DocumentMut) -> Option<(PathBuf, String)> {
    let arr = doc.get("providers")?.as_array_of_tables()?;
    for table in arr.iter() {
        let kind = table.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        if !kind.eq_ignore_ascii_case("runtimed") {
            continue;
        }
        if table.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        let raw = table.get("base_url").and_then(|u| u.as_str())?;
        let socket = PathBuf::from(raw.strip_prefix("varlink:").unwrap_or(raw));
        if !socket.exists() {
            continue;
        }
        return Some((socket, first_model_name(table)?));
    }
    None
}

/// One single-token Generate against the engine so its weights are
/// resident before the first real question. Best-effort: a failure
/// warns and setup still succeeds (asks just pay the load then).
pub(crate) async fn warmup_engine(doc: &mut DocumentMut, report: &mut Vec<(String, String)>) {
    let Some((socket, model)) = warmup_target(doc) else {
        println!("  skipped (no live engine registered)");
        return;
    };
    println!("  loading {} (one-time, a minute or two)...", model.bold());
    let params = serde_json::json!({
        "model": model,
        "prompt": "hi",
        "max_tokens": 1,
        "temperature": 0.0,
        "top_k": 0,
        "top_p": 1.0,
        "seed": 1,
    });
    match RouterctlClient::varlink_call_path(
        &socket,
        "io.syntrop.Runtime1.Generate",
        params,
        WARMUP_TIMEOUT,
    )
    .await
    {
        Ok(_) => {
            println!("  {} engine warm", "LIVE ·".green().bold());
            report.push(("engine-warmup".to_string(), "LIVE · weights resident".to_string()));
        }
        Err(e) => {
            println!(
                "  {} warmup failed ({}); first question pays the load",
                "OFF ·".red().bold(),
                e
            );
            report.push(("engine-warmup".to_string(), format!("OFF · {e}")));
        }
    }
}

pub(crate) fn reload_service() {
    println!();
    println!("Reloading routerd service...");
    let output = std::process::Command::new("systemctl")
        .args(["restart", "routerd"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            println!(
                "{}",
                "  [✓] Successfully reloaded routerd service via systemctl.".green().bold()
            );
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            println!(
                "{}",
                format!(
                    "  [!] Note: could not restart routerd.service ({}). Run 'sudo systemctl restart routerd' if needed.",
                    stderr.trim()
                )
                .yellow()
            );
        }
        Err(e) => {
            println!(
                "{}",
                format!("  [!] Note: systemctl not executed ({}).", e).yellow()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::setup::edit_config::set_models_array;
    use toml_edit::{ArrayOfTables, Item, Value};

fn warmup_doc(socket: &str, kind: &str, enabled: bool) -> DocumentMut {
    let mut doc = DocumentMut::new();
    doc["providers"] = Item::ArrayOfTables(ArrayOfTables::new());
    let arr = doc["providers"].as_array_of_tables_mut().unwrap();
    let mut table = Table::new();
    table.insert("id", Item::Value(Value::from("runtimed-local")));
    table.insert("kind", Item::Value(Value::from(kind)));
    table.insert("base_url", Item::Value(Value::from(socket)));
    table.insert("enabled", Item::Value(Value::from(enabled)));
    set_models_array(&mut table, &["gemma".to_string()]);
    arr.push(table);
    doc
}

#[test]
fn test_warmup_target_needs_live_runtimed_row() {
    // A real file stands in for the socket (existence is the check).
    let sock = std::env::temp_dir().join(format!("warmup-test-{}.sock", std::process::id()));
    std::fs::write(&sock, b"").unwrap();
    let hit = warmup_target(&warmup_doc(sock.to_str().unwrap(), "runtimed", true)).unwrap();
    assert_eq!(hit.1, "gemma");
    assert!(warmup_target(&warmup_doc(sock.to_str().unwrap(), "runtimed", false)).is_none());
    assert!(warmup_target(&warmup_doc(sock.to_str().unwrap(), "openai", true)).is_none());
    assert!(warmup_target(&warmup_doc("/nonexistent/warmup.sock", "runtimed", true)).is_none());
    let _ = std::fs::remove_file(&sock);
}
}
