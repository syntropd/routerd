use anyhow::{anyhow, Result};
use std::fs;
use std::io;
use std::path::Path;
use toml_edit::{Array, DocumentMut, Item, Table, Value};

pub(crate) fn load_or_init_config(path: &Path) -> Result<DocumentMut> {
    if path.exists() {
        let content = fs::read_to_string(path).map_err(|e| {
            anyhow!("Failed to read configuration file '{}': {}", path.display(), e)
        })?;
        let doc = content.parse::<DocumentMut>().map_err(|e| {
            anyhow!("Failed to parse configuration TOML '{}': {}", path.display(), e)
        })?;
        Ok(doc)
    } else {
        let template = routerd_core::DEFAULT_ROUTERD_TOML;
        let doc = template.parse::<DocumentMut>().map_err(|e| {
            anyhow!("Failed to parse default template configuration: {}", e)
        })?;
        Ok(doc)
    }
}

pub(crate) fn save_config(path: &Path, doc: &DocumentMut) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| {
                anyhow!("Failed to create directory '{}': {}", parent.display(), e)
            })?;
        }
    }

    let toml_string = doc.to_string();
    fs::write(path, toml_string).map_err(|e| {
        if e.kind() == io::ErrorKind::PermissionDenied {
            anyhow!(
                "Permission denied writing to '{}'. Please rerun with sudo: sudo routerctl setup",
                path.display()
            )
        } else {
            anyhow!("Failed to write configuration to '{}': {}", path.display(), e)
        }
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o640);
        let _ = fs::set_permissions(path, perms);
        apply_root_syntrop_ownership(path);
    }

    Ok(())
}

pub fn ensure_thresholds_config(doc: &mut DocumentMut) {
    if !doc.contains_table("thresholds") && !doc.contains_key("thresholds") {
        let mut table = Table::new();
        table.insert("max_latency_ms", Item::Value(Value::from(15000)));
        table.insert("psi_memory_threshold", Item::Value(Value::from(25.0)));
        table.insert("max_retries", Item::Value(Value::from(2)));
        table.insert("rss_limit_mb", Item::Value(Value::from(15)));
        table.insert("min_tokens_per_second", Item::Value(Value::from(10.0)));
        doc.insert("thresholds", Item::Table(table));
    } else if let Some(item) = doc.get_mut("thresholds") {
        if let Some(t) = item.as_table_like_mut() {
            if !t.contains_key("min_tokens_per_second") {
                t.insert("min_tokens_per_second", Item::Value(Value::from(10.0)));
            }
        }
    }
}

/// Overwrite a provider table's models list with plain verified names.
pub(crate) fn set_models_array(table: &mut Table, models: &[String]) {
    let mut arr = Array::new();
    for m in models {
        arr.push(m.as_str());
    }
    table.insert("models", Item::Value(Value::Array(arr)));
}

fn apply_root_syntrop_ownership(path: &Path) {
    #[cfg(unix)]
    {
        let is_root = rustix::process::getuid().as_raw() == 0;
        if is_root {
            if let Some(gid) = get_syntrop_gid() {
                let _ = std::os::unix::fs::chown(path, Some(0), Some(gid));
            }
        }
    }
}

fn get_syntrop_gid() -> Option<u32> {
    if let Ok(content) = fs::read_to_string("/etc/group") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 && parts[0] == "syntrop" {
                if let Ok(gid) = parts[2].parse::<u32>() {
                    return Some(gid);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

#[test]
fn test_ensure_thresholds_config() {
    let mut empty_doc = DocumentMut::new();
    ensure_thresholds_config(&mut empty_doc);
    let s = empty_doc.to_string();
    assert!(s.contains("[thresholds]"));
    assert!(s.contains("min_tokens_per_second = 10.0"));

    let mut existing_thresholds = r#"
    [thresholds]
    max_latency_ms = 8000
    "#
    .parse::<DocumentMut>()
    .unwrap();
    ensure_thresholds_config(&mut existing_thresholds);
    let s2 = existing_thresholds.to_string();
    assert!(s2.contains("max_latency_ms = 8000"));
    assert!(s2.contains("min_tokens_per_second = 10.0"));

    let mut custom_thresholds = r#"
    [thresholds]
    min_tokens_per_second = 33.3
    "#
    .parse::<DocumentMut>()
    .unwrap();
    ensure_thresholds_config(&mut custom_thresholds);
    let s3 = custom_thresholds.to_string();
    assert!(s3.contains("min_tokens_per_second = 33.3"));
}
}
