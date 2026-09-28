use crate::client::RouterctlClient;
use anyhow::Result;
use colored::Colorize;
use tabled::settings::Style;
use tabled::{Table, Tabled};

#[derive(Tabled)]
struct ProviderTableRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Kind")]
    kind: String,
    #[tabled(rename = "Tier")]
    tier: String,
    #[tabled(rename = "Weight")]
    weight: String,
    #[tabled(rename = "Health")]
    health: String,
    #[tabled(rename = "Latency")]
    latency: String,
    #[tabled(rename = "Requests")]
    requests: String,
    #[tabled(rename = "Models")]
    models: String,
}

pub async fn run_list(client: &RouterctlClient, json_output: bool) -> Result<()> {
    let res = client.list_providers().await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&res)?);
        return Ok(());
    }

    let providers = res
        .get("providers")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();

    if providers.is_empty() {
        println!("{}", "No providers currently registered.".yellow());
        return Ok(());
    }

    let mut rows = Vec::new();
    for p in providers {
        let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let kind = p.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let tier = p.get("tier").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let weight = format!("{:.2}", p.get("weight").and_then(|v| v.as_f64()).unwrap_or(1.0));
        let is_healthy = p.get("is_healthy").and_then(|v| v.as_bool()).unwrap_or(false);
        let health = if is_healthy { "HEALTHY".to_string() } else { "OFFLINE".to_string() };
        let latency = format!(
            "{:.1}ms",
            p.get("last_latency_ms").and_then(|v| v.as_f64()).unwrap_or(0.0)
        );
        let requests = format!(
            "{}",
            p.get("total_requests").and_then(|v| v.as_u64()).unwrap_or(0)
        );
        let models_arr = p.get("models").and_then(|m| m.as_array()).cloned().unwrap_or_default();
        let models_str = models_arr
            .iter()
            .filter_map(|m| m.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        rows.push(ProviderTableRow {
            id,
            kind,
            tier,
            weight,
            health,
            latency,
            requests,
            models: models_str,
        });
    }

    let mut table = Table::new(rows);
    table.with(Style::rounded());
    println!("{}", table);

    Ok(())
}

pub async fn run_test(
    client: &RouterctlClient,
    provider_id: Option<String>,
    json_output: bool,
) -> Result<()> {
    let providers_to_test = if let Some(id) = provider_id {
        vec![id]
    } else {
        let res = client.list_providers().await?;
        let arr = res
            .get("providers")
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();
        arr.iter()
            .filter_map(|p| p.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
            .collect()
    };

    let mut results = Vec::new();

    for id in providers_to_test {
        match client.test_provider(&id).await {
            Ok(val) => {
                let healthy = val.get("healthy").and_then(|h| h.as_bool()).unwrap_or(false);
                let latency = val.get("latency_ms").and_then(|l| l.as_f64()).unwrap_or(0.0);
                let err = val.get("error").and_then(|e| e.as_str()).unwrap_or("");

                if json_output {
                    results.push(val);
                } else if healthy {
                    println!(
                        "  [{}] Provider '{}' reachable in {:.1}ms",
                        "PASS".green().bold(),
                        id.bold(),
                        latency
                    );
                } else {
                    println!(
                        "  [{}] Provider '{}' probe failed: {}",
                        "FAIL".red().bold(),
                        id.bold(),
                        err.yellow()
                    );
                }
            }
            Err(e) => {
                if !json_output {
                    println!(
                        "  [{}] Provider '{}' error: {}",
                        "ERR".red().bold(),
                        id.bold(),
                        e
                    );
                }
            }
        }
    }

    if json_output {
        println!("{}", serde_json::to_string_pretty(&results)?);
    }

    Ok(())
}
