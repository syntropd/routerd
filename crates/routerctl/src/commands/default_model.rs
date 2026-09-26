use crate::cli::DefaultArgs;
use crate::commands::setup::{live_models, reload_service, save_config, set_tier_default};
use anyhow::{anyhow, Result};
use colored::Colorize;
use std::fs;
use toml_edit::DocumentMut;

fn tier_default(doc: &DocumentMut, tier: &str) -> Option<String> {
    doc.get("tiers")
        .and_then(|t| t.get(tier))
        .and_then(|t| t.get("default_model"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn show_current(doc: &DocumentMut, json_output: bool) -> Result<()> {
    let fast = tier_default(doc, "fast");
    let hard = tier_default(doc, "hard");
    if json_output {
        println!(
            "{}",
            serde_json::json!({ "fast": fast, "hard": hard })
        );
        return Ok(());
    }
    match (fast, hard) {
        (Some(f), Some(h)) if f == h => println!("default → {}", f.bold()),
        (f, h) => {
            println!("fast: {}", f.as_deref().unwrap_or("(scoring decides)"));
            println!("hard: {}", h.as_deref().unwrap_or("(scoring decides)"));
        }
    }
    Ok(())
}

pub fn run_default(args: &DefaultArgs, json_output: bool) -> Result<()> {
    if !args.config.exists() {
        return Err(anyhow!(
            "No config at {}. Run: sudo routerctl setup",
            args.config.display()
        ));
    }
    let content = fs::read_to_string(&args.config).map_err(|e| {
        anyhow!("Failed to read '{}': {}", args.config.display(), e)
    })?;
    let mut doc: DocumentMut = content
        .parse()
        .map_err(|e| anyhow!("Failed to parse '{}': {}", args.config.display(), e))?;

    let Some(want) = args.model.as_deref() else {
        return show_current(&doc, json_output);
    };

    let live = live_models(&doc);
    let Some(canonical) = live
        .iter()
        .find(|(_, name)| name.eq_ignore_ascii_case(want))
        .map(|(_, name)| name.clone())
    else {
        let names: Vec<&str> = live.iter().map(|(_, n)| n.as_str()).collect();
        return Err(anyhow!(
            "'{}' is not a connected model.{}",
            want,
            if names.is_empty() {
                " Run: sudo routerctl setup".to_string()
            } else {
                format!(" Connected: {}", names.join(", "))
            }
        ));
    };

    set_tier_default(&mut doc, "fast", &canonical);
    set_tier_default(&mut doc, "hard", &canonical);
    save_config(&args.config, &doc)?;
    if !args.no_reload {
        reload_service();
    }
    println!("default → {} (fast + hard)", canonical.bold());
    Ok(())
}
