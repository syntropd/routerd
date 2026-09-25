use crate::cli::TestArgs;
use crate::client::RouterctlClient;
use anyhow::Result;
use colored::Colorize;

pub async fn run_benchmark(
    client: &RouterctlClient,
    args: TestArgs,
    json_output: bool,
) -> Result<()> {
    if !json_output {
        println!(
            "Benchmarking route for model '{}' (tier: {:?})...",
            args.model.bold(),
            args.tier
        );
    }

    match client.test_benchmark(&args.model, &args.prompt, args.tier.as_deref()).await {
        Ok((latency_ms, response_sample)) => {
            if json_output {
                let obj = serde_json::json!({
                    "model": args.model,
                    "latency_ms": latency_ms,
                    "response": response_sample,
                    "success": true
                });
                println!("{}", serde_json::to_string_pretty(&obj)?);
            } else {
                println!(
                    "{} Latency: {:.1} ms",
                    "Success:".bold().green(),
                    latency_ms
                );
                if !response_sample.is_empty() {
                    let preview = if response_sample.len() > 120 {
                        format!("{}...", &response_sample[..120])
                    } else {
                        response_sample
                    };
                    println!("  Response preview: \"{}\"", preview.dimmed());
                }
            }
        }
        Err(e) => {
            if json_output {
                let obj = serde_json::json!({
                    "model": args.model,
                    "success": false,
                    "error": e.to_string()
                });
                println!("{}", serde_json::to_string_pretty(&obj)?);
            } else {
                println!("{} {}", "Benchmark failed:".bold().red(), e);
            }
        }
    }

    Ok(())
}
