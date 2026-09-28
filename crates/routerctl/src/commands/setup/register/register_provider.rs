use super::super::edit_config::set_models_array;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value};

/// Socket proving the owned runtimed engine is alive.
pub(crate) const RUNTIMED_SOCKET: &str = "/run/syntrop/io.syntrop.Runtime1";

/// Insert or update a provider by id. Re-running setup never duplicates.
pub fn add_custom_provider(
    doc: &mut DocumentMut,
    id: &str,
    name: &str,
    kind: &str,
    base_url: &str,
    api_key: Option<&str>,
    models: &[String],
    enabled: bool,
) {
    let providers = doc.entry("providers").or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    let arr = match providers.as_array_of_tables_mut() {
        Some(a) => a,
        None => return,
    };

    for table in arr.iter_mut() {
        let id_str = table
            .get("id")
            .and_then(|i| i.as_str())
            .or_else(|| table.get("name").and_then(|n| n.as_str()));
        if id_str == Some(id) {
            table.insert("name", Item::Value(Value::from(if name.is_empty() { id } else { name })));
            table.insert("kind", Item::Value(Value::from(kind)));
            table.insert("base_url", Item::Value(Value::from(base_url)));
            if let Some(key) = api_key {
                if !key.is_empty() {
                    table.insert("api_key", Item::Value(Value::from(key)));
                }
            }
            set_models_array(table, models);
            table.insert("enabled", Item::Value(Value::from(enabled)));
            return;
        }
    }

    let mut table = Table::new();
    table.insert("id", Item::Value(Value::from(id)));
    table.insert("name", Item::Value(Value::from(if name.is_empty() { id } else { name })));
    table.insert("kind", Item::Value(Value::from(kind)));
    table.insert("base_url", Item::Value(Value::from(base_url)));
    if let Some(key) = api_key {
        if !key.is_empty() {
            table.insert("api_key", Item::Value(Value::from(key)));
        }
    }
    table.insert("tier", Item::Value(Value::from("fast")));
    table.insert("weight", Item::Value(Value::from(1.0)));
    table.insert("enabled", Item::Value(Value::from(enabled)));
    table.insert("timeout_ms", Item::Value(Value::from(30000)));
    set_models_array(&mut table, models);
    arr.push(table);
}

/// Register real on-disk model files under the local varlink provider.
/// Merges with whatever is already there; never duplicates.
/// Register GGUF files under the owned runtimed engine.
pub fn upsert_runtimed_models(doc: &mut DocumentMut, names: &[String], enabled: bool) {
    upsert_provider_models(
        doc,
        names,
        enabled,
        "runtimed-local",
        "Runtimed Owned Engine",
        "runtimed",
        RUNTIMED_SOCKET,
    );
}

#[allow(clippy::too_many_arguments)]
fn upsert_provider_models(
    doc: &mut DocumentMut,
    names: &[String],
    enabled: bool,
    id: &str,
    name: &str,
    kind: &str,
    base_url: &str,
) {
    if names.is_empty() {
        return;
    }

    let providers = doc.entry("providers").or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    let arr = match providers.as_array_of_tables_mut() {
        Some(a) => a,
        None => return,
    };

    let mut found_idx = None;
    for (idx, table) in arr.iter().enumerate() {
        let id_str = table
            .get("id")
            .and_then(|i| i.as_str())
            .or_else(|| table.get("name").and_then(|n| n.as_str()));
        let table_kind = table.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        if id_str == Some(id) || table_kind == kind {
            found_idx = Some(idx);
            break;
        }
    }

    let local_table = if let Some(idx) = found_idx {
        arr.get_mut(idx).unwrap()
    } else {
        let mut table = Table::new();
        table.insert("id", Item::Value(Value::from(id)));
        table.insert("name", Item::Value(Value::from(name)));
        table.insert("kind", Item::Value(Value::from(kind)));
        table.insert("base_url", Item::Value(Value::from(base_url)));
        table.insert("tier", Item::Value(Value::from("fast")));
        table.insert("weight", Item::Value(Value::from(1.3)));
        table.insert("enabled", Item::Value(Value::from(enabled)));
        table.insert("timeout_ms", Item::Value(Value::from(300000)));
        arr.push(table);
        let last_idx = arr.len() - 1;
        arr.get_mut(last_idx).unwrap()
    };

    local_table.insert("enabled", Item::Value(Value::from(enabled)));

    // Merge names into whatever models shape is already there.
    if let Some(models_item) = local_table.get_mut("models") {
        if let Some(arr_tables) = models_item.as_array_of_tables_mut() {
            for name in names {
                let exists = arr_tables.iter().any(|t| {
                    t.get("name").and_then(|n| n.as_str()) == Some(name.as_str())
                });
                if !exists {
                    let mut m_tab = Table::new();
                    m_tab.insert("name", Item::Value(Value::from(name.as_str())));
                    arr_tables.push(m_tab);
                }
            }
        } else if let Some(arr_val) = models_item.as_array_mut() {
            for name in names {
                let exists = arr_val.iter().any(|v| v.as_str() == Some(name.as_str()));
                if !exists {
                    arr_val.push(name.as_str());
                }
            }
        }
    } else {
        set_models_array(local_table, names);
    }
}

/// Release every model name a provider claims, leaving the row itself.
/// Used when names move to the engine that actually serves them.
pub(crate) fn clear_provider_models(doc: &mut DocumentMut, id: &str) {
    let Some(arr) = doc
        .get_mut("providers")
        .and_then(|p| p.as_array_of_tables_mut())
    else {
        return;
    };
    for table in arr.iter_mut() {
        let id_str = table
            .get("id")
            .and_then(|i| i.as_str())
            .or_else(|| table.get("name").and_then(|n| n.as_str()));
        if id_str == Some(id) {
            table.insert("models", Item::Value(Value::Array(Array::new())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

#[test]
fn test_upsert_runtimed_models() {
    let mut doc = DocumentMut::new();
    doc["providers"] = Item::ArrayOfTables(ArrayOfTables::new());
    upsert_runtimed_models(&mut doc, &["m1".to_string()], true);
    upsert_runtimed_models(&mut doc, &["m1".to_string(), "m2".to_string()], true);
    let s = doc.to_string();
    assert_eq!(s.matches("\"m1\"").count(), 1);
    assert!(s.contains("\"m2\""));
    assert!(s.contains("id = \"runtimed-local\""));
    assert!(s.contains("kind = \"runtimed\""));
    assert!(s.contains("/run/syntrop/io.syntrop.Runtime1"));
    assert!(s.contains("timeout_ms = 300000"));
}

#[test]
fn test_clear_provider_models_releases_names() {
    let mut doc = DocumentMut::new();
    doc["providers"] = Item::ArrayOfTables(ArrayOfTables::new());
    upsert_runtimed_models(&mut doc, &["m1".to_string()], true);
    // Legacy broker row holding a stale claim.
    let arr = doc["providers"].as_array_of_tables_mut().unwrap();
    let mut legacy = Table::new();
    legacy.insert("id", Item::Value(Value::from("syntrop-local")));
    set_models_array(&mut legacy, &["m1".to_string()]);
    arr.push(legacy);
    clear_provider_models(&mut doc, "syntrop-local");
    let s = doc.to_string();
    // m1 survives exactly once: under the engine, not the broker.
    assert_eq!(s.matches("\"m1\"").count(), 1);
    assert!(s.contains("id = \"runtimed-local\""));
}
}
