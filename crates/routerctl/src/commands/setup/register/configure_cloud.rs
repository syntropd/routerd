use super::super::edit_config::set_models_array;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};

// Dormant in local-only mode; kept (and unit-tested) for cloud restore.
#[allow(dead_code)]
const MINIMAX_BASE_URL: &str = "https://api.minimaxi.chat/v1";
#[allow(dead_code)]
const MISTRAL_BASE_URL: &str = "https://api.mistral.ai/v1";

/// Dormant in local-only mode; kept for cloud restore.
#[allow(dead_code)]
pub fn configure_minimax(
    doc: &mut DocumentMut,
    api_key: Option<String>,
    models: Option<Vec<String>>,
) {
    let providers = doc.entry("providers").or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    let arr = match providers.as_array_of_tables_mut() {
        Some(a) => a,
        None => return,
    };

    let mut found = false;
    for table in arr.iter_mut() {
        let id_str = table
            .get("id")
            .and_then(|i| i.as_str())
            .or_else(|| table.get("name").and_then(|n| n.as_str()));
        if id_str == Some("minimax") {
            found = true;
            match &api_key {
                Some(key) => {
                    table.insert("base_url", Item::Value(Value::from(MINIMAX_BASE_URL)));
                    table.insert("api_key", Item::Value(Value::from(key.as_str())));
                    if let Some(ids) = &models {
                        set_models_array(table, ids);
                    }
                    table.insert("enabled", Item::Value(Value::from(true)));
                }
                None => {
                    table.insert("enabled", Item::Value(Value::from(false)));
                }
            }
            break;
        }
    }

    if !found {
        if let Some(key) = &api_key {
            let mut table = Table::new();
            table.insert("id", Item::Value(Value::from("minimax")));
            table.insert("name", Item::Value(Value::from("MiniMax AI Cloud")));
            table.insert("kind", Item::Value(Value::from("minimax")));
            table.insert("base_url", Item::Value(Value::from(MINIMAX_BASE_URL)));
            table.insert("api_key", Item::Value(Value::from(key.as_str())));
            table.insert("enabled", Item::Value(Value::from(true)));
            table.insert("tier", Item::Value(Value::from("hard")));
            table.insert("weight", Item::Value(Value::from(1.0)));
            table.insert("timeout_ms", Item::Value(Value::from(30000)));
            set_models_array(&mut table, &models.unwrap_or_default());
            arr.push(table);
        }
    }
}

/// Dormant in local-only mode; kept for cloud restore.
#[allow(dead_code)]
pub fn configure_mistral(
    doc: &mut DocumentMut,
    api_key: Option<String>,
    models: Option<Vec<String>>,
) {
    let providers = doc.entry("providers").or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    let arr = match providers.as_array_of_tables_mut() {
        Some(a) => a,
        None => return,
    };

    let mut found = false;
    for table in arr.iter_mut() {
        let id_str = table
            .get("id")
            .and_then(|i| i.as_str())
            .or_else(|| table.get("name").and_then(|n| n.as_str()));
        if id_str == Some("mistral") {
            found = true;
            match &api_key {
                Some(key) => {
                    table.insert("base_url", Item::Value(Value::from(MISTRAL_BASE_URL)));
                    table.insert("api_key", Item::Value(Value::from(key.as_str())));
                    if let Some(ids) = &models {
                        set_models_array(table, ids);
                    }
                    table.insert("enabled", Item::Value(Value::from(true)));
                }
                None => {
                    table.insert("enabled", Item::Value(Value::from(false)));
                }
            }
            break;
        }
    }

    if !found {
        if let Some(key) = &api_key {
            let mut table = Table::new();
            table.insert("id", Item::Value(Value::from("mistral")));
            table.insert("name", Item::Value(Value::from("Mistral AI Platform")));
            table.insert("kind", Item::Value(Value::from("openai")));
            table.insert("base_url", Item::Value(Value::from(MISTRAL_BASE_URL)));
            table.insert("api_key", Item::Value(Value::from(key.as_str())));
            table.insert("enabled", Item::Value(Value::from(true)));
            table.insert("tier", Item::Value(Value::from("hard")));
            table.insert("weight", Item::Value(Value::from(1.0)));
            table.insert("timeout_ms", Item::Value(Value::from(25000)));
            set_models_array(&mut table, &models.unwrap_or_default());
            arr.push(table);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::register_provider::add_custom_provider;

#[test]
fn test_minimax_mistral_custom_config_edit() {
    let template = routerd_core::DEFAULT_ROUTERD_TOML;
    let mut doc = template.parse::<DocumentMut>().unwrap();

    // MiniMax enable with key and verified models
    configure_minimax(
        &mut doc,
        Some("unit-test-key-minimax".to_string()),
        Some(vec!["live-model-a".to_string()]),
    );
    let s = doc.to_string();
    assert!(s.contains("unit-test-key-minimax"));
    assert!(s.contains("live-model-a"));

    // Mistral enable then disable (dormant cloud path, kept tested)
    configure_mistral(
        &mut doc,
        Some("unit-test-key-mistral".to_string()),
        Some(vec!["m-live".to_string()]),
    );
    configure_mistral(&mut doc, None, None);
    let s2 = doc.to_string();
    let doc2: DocumentMut = s2.parse().unwrap();
    let mistral = doc2["providers"]
        .as_array_of_tables()
        .unwrap()
        .iter()
        .find(|t| t.get("id").and_then(|v| v.as_str()) == Some("mistral"))
        .unwrap();
    assert_eq!(
        mistral.get("enabled").and_then(|v| v.as_bool()),
        Some(false)
    );

    // Add custom provider twice: must upsert, never duplicate
    for _ in 0..2 {
        add_custom_provider(
            &mut doc,
            "openrouter-fast",
            "OpenRouter Gateway",
            "openai",
            "https://openrouter.ai/api/v1",
            Some("sk-or-test"),
            &["anthropic/claude-3.5-sonnet".to_string()],
            true,
        );
    }
    let s3 = doc.to_string();
    assert_eq!(s3.matches("id = \"openrouter-fast\"").count(), 1);
    assert!(s3.contains("anthropic/claude-3.5-sonnet"));
}
}
