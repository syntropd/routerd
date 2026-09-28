use super::run_setup::prompt_line;
use colored::Colorize;
use toml_edit::{DocumentMut, Item, Table, Value};

/// Enabled providers' models as (provider id, model name) pairs.
pub fn live_models(doc: &DocumentMut) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(arr) = doc.get("providers").and_then(|p| p.as_array_of_tables()) else {
        return out;
    };
    for table in arr.iter() {
        if table.get("enabled").and_then(|v| v.as_bool()) != Some(true) {
            continue;
        }
        let id = table
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        let Some(models) = table.get("models") else { continue };
        if let Some(names) = models.as_array() {
            for v in names.iter().filter_map(|v| v.as_str()) {
                out.push((id.clone(), v.to_string()));
            }
        } else if let Some(tables) = models.as_array_of_tables() {
            for t in tables.iter().filter_map(|t| t.get("name").and_then(|n| n.as_str())) {
                out.push((id.clone(), t.to_string()));
            }
        }
    }
    out
}

/// Accept a 1-based number, an exact model name, or skip/empty.
pub fn parse_default_pick(input: &str, options: &[(String, String)]) -> Option<String> {
    let answer = input.trim();
    if answer.is_empty() || answer.eq_ignore_ascii_case("skip") || answer.eq_ignore_ascii_case("n") {
        return None;
    }
    if let Ok(n) = answer.parse::<usize>() {
        if n >= 1 && n <= options.len() {
            return Some(options[n - 1].1.clone());
        }
        return None;
    }
    options
        .iter()
        .find(|(_, name)| name.eq_ignore_ascii_case(answer))
        .map(|(_, name)| name.clone())
}

/// Write a tier's default model, creating tables as needed.
pub fn set_tier_default(doc: &mut DocumentMut, tier: &str, model: &str) {
    if doc.get("tiers").is_none() {
        doc["tiers"] = Item::Table(Table::new());
    }
    let tiers = &mut doc["tiers"];
    if tiers.get(tier).is_none() {
        tiers[tier] = Item::Table(Table::new());
    }
    tiers[tier]["default_model"] = Item::Value(Value::from(model));
}

/// Non-interactive default: the owned engine's first live model wins;
/// otherwise the first live model. Prints what it chose.
pub(crate) fn auto_default_model(doc: &mut DocumentMut) {
    let options = live_models(doc);
    if options.is_empty() {
        println!("  no verified models; skipping");
        return;
    }
    let pick = options
        .iter()
        .find(|(prov, _)| prov == "runtimed-local")
        .or(options.first())
        .map(|(_, name)| name.clone())
        .unwrap();
    set_tier_default(doc, "fast", &pick);
    set_tier_default(doc, "hard", &pick);
    println!("  default → {} (fast + hard)", pick.bold());
}

pub(crate) fn pick_default_model(doc: &mut DocumentMut) {
    let options = live_models(doc);
    if options.is_empty() {
        println!("  no verified models; skipping");
        return;
    }
    for (i, (prov, name)) in options.iter().enumerate() {
        println!("  {}) {} · {}", i + 1, prov, name.bold());
    }
    let answer = prompt_line(&format!("default [1-{}, name, skip]: ", options.len()));
    match parse_default_pick(&answer, &options) {
        Some(model) => {
            set_tier_default(doc, "fast", &model);
            set_tier_default(doc, "hard", &model);
            println!("  default → {} (fast + hard)", model.bold());
        }
        None => println!("  no default; scoring decides"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::setup::edit_config::set_models_array;
    use crate::commands::setup::register::register_provider::upsert_runtimed_models;
    use toml_edit::ArrayOfTables;

#[test]
fn test_auto_default_prefers_engine() {
    let mut doc = DocumentMut::new();
    doc["providers"] = Item::ArrayOfTables(ArrayOfTables::new());
    // A foreign provider first in file order; the engine must still win.
    let arr = doc["providers"].as_array_of_tables_mut().unwrap();
    let mut cloud = Table::new();
    cloud.insert("id", Item::Value(Value::from("cloud-x")));
    cloud.insert("enabled", Item::Value(Value::from(true)));
    set_models_array(&mut cloud, &["qwen".to_string()]);
    arr.push(cloud);
    upsert_runtimed_models(&mut doc, &["gemma".to_string()], true);
    auto_default_model(&mut doc);
    let s = doc.to_string();
    assert!(s.contains("default_model = \"gemma\""));
    assert!(!s.contains("default_model = \"qwen\""));
}

#[test]
fn test_default_pick_parsing() {
    let options = vec![
        ("prov-a".to_string(), "model-a".to_string()),
        ("prov-b".to_string(), "model-b".to_string()),
    ];
    assert_eq!(parse_default_pick("1", &options), Some("model-a".to_string()));
    assert_eq!(parse_default_pick("2", &options), Some("model-b".to_string()));
    assert_eq!(parse_default_pick("model-b", &options), Some("model-b".to_string()));
    assert_eq!(parse_default_pick("MODEL-A", &options), Some("model-a".to_string()));
    assert_eq!(parse_default_pick("", &options), None);
    assert_eq!(parse_default_pick("skip", &options), None);
    assert_eq!(parse_default_pick("0", &options), None);
    assert_eq!(parse_default_pick("9", &options), None);
    assert_eq!(parse_default_pick("nope", &options), None);
}

#[test]
fn test_live_models_and_tier_default() {
    let mut doc = r#"
[[providers]]
id = "on"
enabled = true
models = ["a", "b"]

[[providers]]
id = "detailed"
enabled = true

[[providers.models]]
name = "d"

[[providers]]
id = "off"
enabled = false
models = ["c"]
"#
    .parse::<DocumentMut>()
    .unwrap();
    let live = live_models(&doc);
    assert_eq!(
        live,
        vec![
            ("on".to_string(), "a".to_string()),
            ("on".to_string(), "b".to_string()),
            ("detailed".to_string(), "d".to_string()),
        ]
    );

    set_tier_default(&mut doc, "fast", "b");
    set_tier_default(&mut doc, "hard", "b");
    assert_eq!(
        doc["tiers"]["fast"]["default_model"].as_str(),
        Some("b")
    );
    assert_eq!(
        doc["tiers"]["hard"]["default_model"].as_str(),
        Some("b")
    );
}
}
