use crate::client::RouterctlClient;
use anyhow::Result;
use colored::Colorize;
use tabled::settings::Style;
use tabled::{Table, Tabled};

#[derive(Tabled)]
struct ModelTableRow {
    #[tabled(rename = "Model Identifier")]
    name: String,
    #[tabled(rename = "Type")]
    kind: String,
}

pub async fn run_models(client: &RouterctlClient, json_output: bool) -> Result<()> {
    let models = client.list_models().await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&models)?);
        return Ok(());
    }

    if models.is_empty() {
        println!("{}", "No models available.".yellow());
        return Ok(());
    }

    let mut rows = Vec::new();
    for m in models {
        let kind = if m.starts_with("router:") || m == "fast" || m == "hard" || m == "auto" {
            "Virtual Tier Route".to_string()
        } else {
            "Hardware / Cloud Model".to_string()
        };
        rows.push(ModelTableRow { name: m, kind });
    }

    let mut table = Table::new(rows);
    table.with(Style::rounded());
    println!("{}", table);

    Ok(())
}
