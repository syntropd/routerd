use crate::client::RouterctlClient;
use anyhow::Result;
use colored::Colorize;

pub async fn run_status(client: &RouterctlClient, json_output: bool) -> Result<()> {
    let status_val = client.get_status().await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&status_val)?);
        return Ok(());
    }

    println!("{}", "=== routerd Daemon Status ===".bold().cyan());

    let status = status_val.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
    let version = status_val.get("version").and_then(|v| v.as_str()).unwrap_or("0.3.0");
    let uptime = status_val.get("uptime_seconds").and_then(|v| v.as_u64()).unwrap_or(0);
    let total_reqs = status_val.get("total_requests").and_then(|v| v.as_u64()).unwrap_or(0);
    let active_reqs = status_val.get("active_requests").and_then(|v| v.as_u64()).unwrap_or(0);
    let providers_count = status_val.get("providers_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let healthy_count = status_val.get("healthy_providers_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let psi_level = status_val.get("psi_level").and_then(|v| v.as_str()).unwrap_or("Normal");
    let psi_mem = status_val.get("psi_memory_some").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let rss_mb = status_val.get("rss_mb").and_then(|v| v.as_f64()).unwrap_or(0.0);

    let status_display = if status == "active" || status == "healthy" {
        status.green().bold()
    } else {
        status.red().bold()
    };

    let psi_display = match psi_level {
        "Critical" => psi_level.red().bold(),
        "Elevated" => psi_level.yellow().bold(),
        _ => psi_level.green(),
    };

    println!("  {:<24} {}", "Service State:".bold(), status_display);
    println!("  {:<24} {}", "Version:".bold(), version);
    println!("  {:<24} {} seconds", "Uptime:".bold(), uptime);
    println!("  {:<24} {}", "Total Requests:".bold(), total_reqs);
    println!("  {:<24} {}", "Active Requests:".bold(), active_reqs);
    println!(
        "  {:<24} {}/{} healthy",
        "Providers:".bold(),
        healthy_count.to_string().green(),
        providers_count
    );
    println!(
        "  {:<24} {} (memory_some={:.1}%)",
        "Kernel PSI Pressure:".bold(),
        psi_display,
        psi_mem
    );
    if rss_mb > 0.0 {
        let rss_colored = if rss_mb > 15.0 {
            format!("{:.2} MB (threshold > 15MB)", rss_mb).red()
        } else {
            format!("{:.2} MB", rss_mb).green()
        };
        println!("  {:<24} {}", "Memory RSS:".bold(), rss_colored);
    }

    Ok(())
}
