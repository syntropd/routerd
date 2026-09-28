use crate::client::RouterctlClient;
use anyhow::Result;
use colored::Colorize;
use tabled::settings::Style;
use tabled::{Table, Tabled};

#[derive(Tabled)]
struct ModelTableRow {
    #[tabled(rename = "Connected Model")]
    name: String,
}

/// Routing aliases. They stay valid request targets (the daemon still serves
/// and routes them), but `models` answers "what is connected", so they are
/// hidden here to avoid confusion with real provider models.
pub fn is_virtual_model(name: &str) -> bool {
    name.starts_with("router:") || name == "fast" || name == "hard" || name == "auto"
}

/// Connected provider models only; virtual routing aliases filtered out.
pub fn connected_models(models: &[String]) -> Vec<String> {
    models
        .iter()
        .filter(|m| !is_virtual_model(m))
        .cloned()
        .collect()
}

pub async fn run_models(client: &RouterctlClient, json_output: bool) -> Result<()> {
    let models = connected_models(&client.list_models().await?);

    if json_output {
        println!("{}", serde_json::to_string_pretty(&models)?);
        return Ok(());
    }

    if models.is_empty() {
        println!("{}", "No models connected. Run: sudo routerctl setup".yellow());
        return Ok(());
    }

    let rows: Vec<ModelTableRow> = models
        .into_iter()
        .map(|m| ModelTableRow { name: m })
        .collect();

    let mut table = Table::new(rows);
    table.with(Style::rounded());
    println!("{}", table);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_aliases_filtered_everywhere() {
        let all = vec![
            "router:fast".to_string(),
            "router:hard".to_string(),
            "router:auto".to_string(),
            "fast".to_string(),
            "hard".to_string(),
            "auto".to_string(),
            "MiniMax-Text-01".to_string(),
        ];
        assert_eq!(connected_models(&all), vec!["MiniMax-Text-01".to_string()]);
        assert!(connected_models(&[]).is_empty());
    }
}
