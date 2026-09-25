use crate::cli::RouteArgs;
use crate::client::RouterctlClient;
use anyhow::Result;
use colored::Colorize;
use tabled::settings::Style;
use tabled::{Table, Tabled};

#[derive(Tabled)]
struct RouteCandidateRow {
    #[tabled(rename = "Rank")]
    rank: usize,
    #[tabled(rename = "Provider")]
    provider: String,
    #[tabled(rename = "Model")]
    model: String,
    #[tabled(rename = "Total Score")]
    score: String,
    #[tabled(rename = "Speed")]
    speed: String,
    #[tabled(rename = "Cost")]
    cost: String,
    #[tabled(rename = "Capability")]
    cap: String,
    #[tabled(rename = "Reason / Notes")]
    reason: String,
}

pub async fn run_route(
    client: &RouterctlClient,
    args: RouteArgs,
    json_output: bool,
) -> Result<()> {
    let res = client
        .route_request(
            Some(&args.model),
            args.tier.as_deref(),
            Some(args.tokens),
            args.stream,
        )
        .await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&res)?);
        return Ok(());
    }

    println!(
        "{} model='{}', tier='{}', tokens={}",
        "=== Routing Simulation ===".bold().cyan(),
        args.model.bold(),
        args.tier.as_deref().unwrap_or("auto").bold(),
        args.tokens
    );

    let candidates = res
        .get("candidates")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    if candidates.is_empty() {
        println!("{}", "No eligible candidate providers found for request.".red());
        return Ok(());
    }

    let mut rows = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        let provider = c.get("provider_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let model = c.get("model_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let score = format!("{:.2}", c.get("total_score").and_then(|v| v.as_f64()).unwrap_or(0.0));
        let speed = format!("{:.1}", c.get("speed_score").and_then(|v| v.as_f64()).unwrap_or(0.0));
        let cost = format!("{:.1}", c.get("cost_score").and_then(|v| v.as_f64()).unwrap_or(0.0));
        let cap = format!("{:.1}", c.get("capability_score").and_then(|v| v.as_f64()).unwrap_or(0.0));
        let reason = c.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string();

        rows.push(RouteCandidateRow {
            rank: i + 1,
            provider,
            model,
            score,
            speed,
            cost,
            cap,
            reason,
        });
    }

    let mut table = Table::new(rows);
    table.with(Style::rounded());
    println!("{}", table);

    if let Some(winner) = candidates.first() {
        let win_prov = winner.get("provider_id").and_then(|v| v.as_str()).unwrap_or("");
        let win_model = winner.get("model_name").and_then(|v| v.as_str()).unwrap_or("");
        println!(
            "{} Candidate selected: {} via {}",
            "Selected Route:".bold().green(),
            win_model.bold(),
            win_prov.bold()
        );
    }

    Ok(())
}
