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
        Ok(bench) => {
            if json_output {
                let obj = serde_json::json!({
                    "model": bench.model,
                    "ttft_ms": bench.ttft_ms,
                    "total_latency_ms": bench.total_latency_ms,
                    "chunks": bench.chunks,
                    "response": bench.content,
                    "success": true
                });
                println!("{}", serde_json::to_string_pretty(&obj)?);
            } else {
                println!(
                    "{} TTFT: {:.1} ms | Total Latency: {:.1} ms (chunks: {})",
                    "Success:".bold().green(),
                    bench.ttft_ms,
                    bench.total_latency_ms,
                    bench.chunks
                );
                if !bench.content.is_empty() {
                    let preview = if bench.content.len() > 120 {
                        format!("{}...", &bench.content[..120])
                    } else {
                        bench.content
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
