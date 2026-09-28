use super::audit_providers::{audit_existing_providers, autodetect_local};
use super::choose_default::{auto_default_model, pick_default_model};
use super::edit_config::{ensure_thresholds_config, load_or_init_config, save_config};
use super::probe_models::fetch_model_ids;
use super::register::add_custom_provider;
use super::warmup_engine::{reload_service, warmup_engine};
use crate::cli::SetupArgs;
use anyhow::Result;
use colored::Colorize;
use routerd_core::config::is_localhost_url;
use std::io::{self, Write};
use toml_edit::DocumentMut;

// Setup registers only servers and model files it verifies live.
// There is deliberately no model-download step: bring GGUF files,
// re-run setup, and they get picked up.

pub(crate) fn prompt_line(prompt: &str) -> String {
    print!("{}", prompt);
    let _ = io::stdout().flush();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_ok() {
        input.trim().to_string()
    } else {
        String::new()
    }
}

pub async fn run_setup(args: &SetupArgs) -> Result<()> {
    println!("{}", "routerctl setup — local models only".bold());
    println!("config {} · models {}", args.config.display(), args.models_dir.display());
    println!();

    let mut doc = load_or_init_config(&args.config)?;
    ensure_thresholds_config(&mut doc);
    let mut report: Vec<(String, String)> = Vec::new();

    println!("{}", "configured".cyan());
    audit_existing_providers(&mut doc, &mut report).await;
    println!();

    println!("{}", "local".cyan());
    autodetect_local(&mut doc, &args.models_dir, &mut report).await;
    println!();

    println!("{}", "custom".cyan());
    if args.auto {
        println!("  skipped (--auto)");
    } else {
        run_custom_step(&mut doc, &mut report).await;
    }
    println!();

    println!("{}", "default".cyan());
    if args.auto {
        auto_default_model(&mut doc);
    } else {
        pick_default_model(&mut doc);
    }
    println!();

    save_config(&args.config, &doc)?;
    println!("saved {}", args.config.display().to_string().bold());
    println!();

    println!("{}", "warming".cyan());
    warmup_engine(&mut doc, &mut report).await;

    if !args.no_reload {
        reload_service();
    }

    print_summary(&report);
    Ok(())
}

/// One custom-provider step: localhost only. Prompt, ping, store only a
/// live answer (or an explicit save-anyway, which stays OFF).
async fn run_custom_step(doc: &mut DocumentMut, report: &mut Vec<(String, String)>) {
    let add_custom = prompt_line("add a custom local provider (vLLM, llama.cpp...)? [y/N]: ");
    if !(add_custom.eq_ignore_ascii_case("y") || add_custom.eq_ignore_ascii_case("yes")) {
        return;
    }
    let p_id = prompt_line("  id [custom-provider]: ");
    let p_id = if p_id.is_empty() { "custom-provider".to_string() } else { p_id };
    let p_name = prompt_line("  name [same as id]: ");
    let p_name = if p_name.is_empty() { p_id.clone() } else { p_name };
    let base_url = prompt_line("  base URL [http://127.0.0.1:8000/v1]: ");
    let base_url = if base_url.is_empty() {
        "http://127.0.0.1:8000/v1".to_string()
    } else {
        base_url.trim_end_matches('/').to_string()
    };
    if !is_localhost_url(&base_url) {
        println!("  {} external URLs are disabled in local-only mode", "OFF ·".red().bold());
        report.push((p_id, "OFF · external (local-only mode)".to_string()));
        return;
    }
    let p_key = prompt_line("  key [none]: ");
    let key_opt = if p_key.is_empty() { None } else { Some(p_key.as_str()) };
    print!("  pinging... ");
    let _ = io::stdout().flush();
    match fetch_model_ids(&base_url, key_opt).await {
        Ok(ids) => {
            println!("{}", format!("LIVE · {} model(s)", ids.len()).green().bold());
            add_custom_provider(doc, &p_id, &p_name, "openai", &base_url, key_opt, &ids, true);
            report.push((p_id, format!("LIVE · {} model(s)", ids.len())));
        }
        Err(e) => {
            println!("{} {}", "OFF ·".red().bold(), e);
            let save = prompt_line("  save anyway (stays OFF)? [y/N]: ");
            if save.eq_ignore_ascii_case("y") || save.eq_ignore_ascii_case("yes") {
                let models_str = prompt_line("  models, comma-separated: ");
                let models: Vec<String> = models_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                add_custom_provider(doc, &p_id, &p_name, "openai", &base_url, key_opt, &models, false);
                report.push((p_id, format!("OFF · {} (saved anyway)", e)));
            } else {
                report.push((p_id, format!("OFF · {} (not saved)", e)));
            }
        }
    }
}

fn print_summary(report: &[(String, String)]) {
    println!("{}", "Verified providers".green().bold());
    if report.is_empty() {
        println!("  (nothing registered)");
    }
    for (id, status) in report {
        println!("  {:<16} {}", id.bold(), status);
    }
    println!();
    println!("models: routerctl models · try it: syn say hello");
}
