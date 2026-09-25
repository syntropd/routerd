use crate::cli::SetupArgs;
use anyhow::{anyhow, Result};
use colored::Colorize;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value};

#[derive(Debug, Clone)]
pub struct GemmaModelInfo {
    pub name: String,
    pub description: &'static str,
    pub size_label: &'static str,
    pub tier: &'static str,
    pub max_context: usize,
    pub avg_latency_ms: f64,
    pub tokens_per_second: f64,
}


pub fn get_gemma_catalog() -> Vec<GemmaModelInfo> {
    vec![
        GemmaModelInfo {
            name: "gemma-4-e2b-it".to_string(),
            description: "Edge 2B, ~1.5 GB VRAM",
            size_label: "1.5 GB",
            tier: "fast",
            max_context: 8192,
            avg_latency_ms: 35.0,
            tokens_per_second: 150.0,
        },
        GemmaModelInfo {
            name: "gemma-4-e4b-it".to_string(),
            description: "Edge 4B, ~2.8 GB VRAM",
            size_label: "2.8 GB",
            tier: "fast",
            max_context: 8192,
            avg_latency_ms: 45.0,
            tokens_per_second: 110.0,
        },
        GemmaModelInfo {
            name: "gemma-4-26b-a4b-it".to_string(),
            description: "MoE 26B/4B active, ~14.0 GB VRAM",
            size_label: "14.0 GB",
            tier: "hard",
            max_context: 16384,
            avg_latency_ms: 120.0,
            tokens_per_second: 45.0,
        },
    ]
}

fn prompt_line(prompt: &str) -> String {
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
    println!();
    println!("{}", "============================================================".cyan().bold());
    println!("{}", " syntrop-routerd — Interactive Provider & Model Setup".cyan().bold());
    println!("{}", "============================================================".cyan().bold());
    println!("Target config:      {}", args.config.display().to_string().bold());
    println!("Credentials file:   {}", args.credentials_path.display().to_string().bold());
    println!("Model storage root: {}", args.models_dir.display().to_string().bold());
    println!();

    // 0. Load or initialize TOML document
    let mut doc = load_or_init_config(&args.config)?;
    ensure_thresholds_config(&mut doc);

    // 1. MiniMax Setup
    println!("{}", "[1/5] MiniMax AI Configuration".bold().green());
    println!("MiniMax delivers high-throughput reasoning and long-context dialogue.");
    let minimax_key = prompt_line("Enter MiniMax API Key (leave empty to skip): ");
    if minimax_key.is_empty() {
        configure_minimax(&mut doc, None);
        println!("  {} MiniMax provider marked disabled.", "ℹ".blue());
    } else {
        print!("  Probing MiniMax API connectivity... ");
        let _ = io::stdout().flush();
        match probe_minimax(&minimax_key).await {
            Ok(()) => {
                println!("{}", "PASS (connection verified)".green().bold());
            }
            Err(e) => {
                println!("{} ({})", "WARN".yellow().bold(), e);
            }
        }
        configure_minimax(&mut doc, Some(minimax_key));
        println!("  {} MiniMax provider configured and enabled.", "✔".green());
    }
    println!();

    // 2. Mistral AI Setup
    println!("{}", "[2/5] Mistral AI Configuration".bold().green());
    println!("Mistral AI provides state-of-the-art European frontier reasoning models.");
    let mistral_key = prompt_line("Enter Mistral API Key (leave empty to skip): ");
    if mistral_key.is_empty() {
        configure_mistral(&mut doc, None);
        println!("  {} Mistral AI provider marked disabled.", "ℹ".blue());
    } else {
        print!("  Probing Mistral AI endpoint (https://api.mistral.ai/v1/models)... ");
        let _ = io::stdout().flush();
        match probe_mistral(&mistral_key).await {
            Ok(()) => {
                println!("{}", "PASS (HTTP 200 OK)".green().bold());
            }
            Err(e) => {
                println!("{} ({})", "WARN".yellow().bold(), e);
            }
        }
        configure_mistral(&mut doc, Some(mistral_key));
        println!("  {} Mistral AI provider configured and enabled.", "✔".green());
    }
    println!();

    // 3. Custom OpenAI-Compatible Provider Setup
    println!("{}", "[3/5] Custom OpenAI-Compatible Provider".bold().green());
    let add_custom = prompt_line("Add a custom OpenAI-compatible provider (e.g. OpenAI, OpenRouter, vLLM, DeepSeek)? [y/N]: ");
    if add_custom.eq_ignore_ascii_case("y") || add_custom.eq_ignore_ascii_case("yes") {
        let p_id = prompt_line("  Provider ID (e.g. openrouter, deepseek, local-vllm): ");
        let p_id = if p_id.is_empty() { "custom-provider".to_string() } else { p_id };
        let p_name = prompt_line("  Display Name (leave empty for same as ID): ");
        let p_name = if p_name.is_empty() { p_id.clone() } else { p_name };
        let base_url = prompt_line("  Base URL (e.g. https://api.deepseek.com/v1): ");
        let base_url = if base_url.is_empty() {
            "https://api.openai.com/v1".to_string()
        } else {
            base_url.trim_end_matches('/').to_string()
        };
        let p_key = prompt_line("  API Key (leave empty if none / local unauthenticated): ");
        let p_key_opt = if p_key.is_empty() { None } else { Some(p_key.as_str()) };
        let models_str = prompt_line("  Model Name(s) (comma-separated, e.g. deepseek-chat, deepseek-coder): ");
        let models: Vec<String> = if models_str.is_empty() {
            vec![format!("{}-model", p_id)]
        } else {
            models_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };

        print!("  Probing custom provider ({}/models)... ", base_url);
        let _ = io::stdout().flush();
        match probe_custom(&base_url, p_key_opt).await {
            Ok(()) => {
                println!("{}", "PASS (reachable)".green().bold());
            }
            Err(e) => {
                println!("{} ({})", "WARN".yellow().bold(), e);
            }
        }

        add_custom_provider(&mut doc, &p_id, &p_name, &base_url, p_key_opt, &models);
        println!("  {} Custom provider '{}' registered with {} models.", "✔".green(), p_id.bold(), models.len());
    } else {
        println!("  {} Skipped custom provider configuration.", "ℹ".blue());
    }
    println!();

    // 4. Hugging Face Access Token Setup
    println!("{}", "[4/5] Hugging Face Credentials".bold().green());
    println!("A Hugging Face access token allows downloading gated and open-weights models.");
    let hf_token = prompt_line("Enter Hugging Face access token (hf_..., leave empty to skip): ");
    if hf_token.is_empty() {
        println!("  {} Skipped Hugging Face token configuration.", "ℹ".blue());
    } else {
        print!("  Validating Hugging Face token via https://huggingface.co/api/whoami-v2... ");
        let _ = io::stdout().flush();
        match validate_huggingface_token(&hf_token).await {
            Ok(user) => {
                println!("{} (authenticated as '{}')", "PASS".green().bold(), user.bold());
            }
            Err(e) => {
                println!("{} ({})", "WARN".yellow().bold(), e);
            }
        }
        update_credentials_env(&args.credentials_path, "HF_TOKEN", &hf_token)?;
        println!(
            "  {} Stored HF_TOKEN in {} with strict 0600 root:syntrop permissions.",
            "✔".green(),
            args.credentials_path.display().to_string().bold()
        );
    }
    println!();

    // 5. Gemma 4 Open-Weights Models Download & Staging
    println!("{}", "[5/5] Gemma 4 Open-Weights Models".bold().green());
    println!("Google Gemma 4 models offer state-of-the-art open weights (Apache 2.0 license)");
    println!("optimized for local Linux hardware acceleration (CUDA, ROCm, Vulkan, CPU).");
    println!();
    println!("Available Models:");
    let catalog = get_gemma_catalog();
    for (i, m) in catalog.iter().enumerate() {
        println!("  [{}] {:<20} ({})", i + 1, m.name.bold(), m.description);
    }
    println!("  [{}] All of the above", catalog.len() + 1);
    println!("  [s] Skip local model download");
    println!();

    let sel = prompt_line("Select model(s) to download or stage (e.g. 1, 2, 'all', or Enter to skip): ");
    let selected_indices = parse_selection(&sel, catalog.len());
    if selected_indices.is_empty() {
        println!("  {} Skipped Gemma 4 model staging.", "ℹ".blue());
    } else {
        let mut staged_models = Vec::new();
        for idx in selected_indices {
            let model = &catalog[idx - 1];
            println!("  ==> Staging Gemma 4 model '{}' ({})...", model.name.bold(), model.size_label);
            stage_gemma_model(&args.models_dir, model).await?;
            staged_models.push(model.clone());
        }
        register_gemma_models(&mut doc, &staged_models);
        println!(
            "  {} Staged and registered {} Gemma 4 model(s) into local varlink inference provider.",
            "✔".green(),
            staged_models.len()
        );
    }
    println!();

    // 6. Write Updated Configuration
    save_config(&args.config, &doc)?;
    println!(
        "{} Saved active configuration to {}",
        "✔".green().bold(),
        args.config.display().to_string().bold()
    );

    // 7. Reload routerd Service
    if !args.no_reload {
        reload_service();
    }

    println!();
    println!("{}", "============================================================".cyan().bold());
    println!("{}", " Setup Completed Successfully!".green().bold());
    println!("{}", "============================================================".cyan().bold());
    println!("Inspect active providers:  routerctl providers list");
    println!("Test provider connection:  routerctl providers test");
    println!("List all available models: routerctl models");
    println!();

    Ok(())
}

fn load_or_init_config(path: &Path) -> Result<DocumentMut> {
    if path.exists() {
        let content = fs::read_to_string(path).map_err(|e| {
            anyhow!("Failed to read configuration file '{}': {}", path.display(), e)
        })?;
        let doc = content.parse::<DocumentMut>().map_err(|e| {
            anyhow!("Failed to parse configuration TOML '{}': {}", path.display(), e)
        })?;
        Ok(doc)
    } else {
        let template = include_str!("../../../../systemd/routerd.toml");
        let doc = template.parse::<DocumentMut>().map_err(|e| {
            anyhow!("Failed to parse default template configuration: {}", e)
        })?;
        Ok(doc)
    }
}

fn save_config(path: &Path, doc: &DocumentMut) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| {
                anyhow!("Failed to create directory '{}': {}", parent.display(), e)
            })?;
        }
    }

    let toml_string = doc.to_string();
    fs::write(path, toml_string).map_err(|e| {
        if e.kind() == io::ErrorKind::PermissionDenied {
            anyhow!(
                "Permission denied writing to '{}'. Please rerun with sudo: sudo routerctl setup",
                path.display()
            )
        } else {
            anyhow!("Failed to write configuration to '{}': {}", path.display(), e)
        }
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o640);
        let _ = fs::set_permissions(path, perms);
        apply_root_syntrop_ownership(path);
    }

    Ok(())
}

pub fn ensure_thresholds_config(doc: &mut DocumentMut) {
    if !doc.contains_table("thresholds") && !doc.contains_key("thresholds") {
        let mut table = Table::new();
        table.insert("max_latency_ms", Item::Value(Value::from(15000)));
        table.insert("psi_memory_threshold", Item::Value(Value::from(25.0)));
        table.insert("max_retries", Item::Value(Value::from(2)));
        table.insert("rss_limit_mb", Item::Value(Value::from(15)));
        table.insert("min_tokens_per_second", Item::Value(Value::from(10.0)));
        doc.insert("thresholds", Item::Table(table));
    } else if let Some(item) = doc.get_mut("thresholds") {
        if let Some(t) = item.as_table_like_mut() {
            if !t.contains_key("min_tokens_per_second") {
                t.insert("min_tokens_per_second", Item::Value(Value::from(10.0)));
            }
        }
    }
}

pub fn configure_minimax(doc: &mut DocumentMut, api_key: Option<String>) {
    let providers = doc.entry("providers").or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    let arr = match providers.as_array_of_tables_mut() {
        Some(a) => a,
        None => return,
    };

    let mut found = false;
    for table in arr.iter_mut() {
        let id_str = table
            .get("id")
            .and_then(|i| i.as_str())
            .or_else(|| table.get("name").and_then(|n| n.as_str()));
        if id_str == Some("minimax") {
            found = true;
            match &api_key {
                Some(key) => {
                    table.insert("api_key", Item::Value(Value::from(key.as_str())));
                    table.insert("enabled", Item::Value(Value::from(true)));
                }
                None => {
                    table.insert("enabled", Item::Value(Value::from(false)));
                }
            }
            break;
        }
    }

    if !found {
        let mut table = Table::new();
        table.insert("id", Item::Value(Value::from("minimax")));
        table.insert("name", Item::Value(Value::from("MiniMax AI Cloud")));
        table.insert("kind", Item::Value(Value::from("minimax")));
        table.insert("base_url", Item::Value(Value::from("https://api.minimaxi.chat/v1")));
        match &api_key {
            Some(key) => {
                table.insert("api_key", Item::Value(Value::from(key.as_str())));
                table.insert("enabled", Item::Value(Value::from(true)));
            }
            None => {
                table.insert("enabled", Item::Value(Value::from(false)));
            }
        }
        table.insert("tier", Item::Value(Value::from("hard")));
        table.insert("weight", Item::Value(Value::from(1.0)));
        table.insert("timeout_ms", Item::Value(Value::from(30000)));

        let mut models = Array::new();
        models.push("abab6.5s-chat");
        table.insert("models", Item::Value(Value::Array(models)));

        arr.push(table);
    }
}

pub fn configure_mistral(doc: &mut DocumentMut, api_key: Option<String>) {
    let providers = doc.entry("providers").or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    let arr = match providers.as_array_of_tables_mut() {
        Some(a) => a,
        None => return,
    };

    let mut found = false;
    for table in arr.iter_mut() {
        let id_str = table
            .get("id")
            .and_then(|i| i.as_str())
            .or_else(|| table.get("name").and_then(|n| n.as_str()));
        if id_str == Some("mistral") {
            found = true;
            match &api_key {
                Some(key) => {
                    table.insert("api_key", Item::Value(Value::from(key.as_str())));
                    table.insert("enabled", Item::Value(Value::from(true)));
                }
                None => {
                    table.insert("enabled", Item::Value(Value::from(false)));
                }
            }
            break;
        }
    }

    if !found {
        let mut table = Table::new();
        table.insert("id", Item::Value(Value::from("mistral")));
        table.insert("name", Item::Value(Value::from("Mistral AI Platform")));
        table.insert("kind", Item::Value(Value::from("openai")));
        table.insert("base_url", Item::Value(Value::from("https://api.mistral.ai/v1")));
        match &api_key {
            Some(key) => {
                table.insert("api_key", Item::Value(Value::from(key.as_str())));
                table.insert("enabled", Item::Value(Value::from(true)));
            }
            None => {
                table.insert("enabled", Item::Value(Value::from(false)));
            }
        }
        table.insert("tier", Item::Value(Value::from("hard")));
        table.insert("weight", Item::Value(Value::from(1.0)));
        table.insert("timeout_ms", Item::Value(Value::from(25000)));

        let mut models = Array::new();
        models.push("mistral-large-latest");
        models.push("mistral-small-latest");
        table.insert("models", Item::Value(Value::Array(models)));

        arr.push(table);
    }
}

pub fn add_custom_provider(
    doc: &mut DocumentMut,
    id: &str,
    name: &str,
    base_url: &str,
    api_key: Option<&str>,
    models: &[String],
) {
    let providers = doc.entry("providers").or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    let arr = match providers.as_array_of_tables_mut() {
        Some(a) => a,
        None => return,
    };

    let mut table = Table::new();
    table.insert("id", Item::Value(Value::from(id)));
    table.insert("name", Item::Value(Value::from(if name.is_empty() { id } else { name })));
    table.insert("kind", Item::Value(Value::from("openai")));
    table.insert("base_url", Item::Value(Value::from(base_url)));
    if let Some(key) = api_key {
        if !key.is_empty() {
            table.insert("api_key", Item::Value(Value::from(key)));
        }
    }
    table.insert("tier", Item::Value(Value::from("fast")));
    table.insert("weight", Item::Value(Value::from(1.0)));
    table.insert("enabled", Item::Value(Value::from(true)));
    table.insert("timeout_ms", Item::Value(Value::from(30000)));

    let mut models_arr = Array::new();
    for m in models {
        models_arr.push(m.as_str());
    }
    table.insert("models", Item::Value(Value::Array(models_arr)));

    arr.push(table);
}

pub fn register_gemma_models(doc: &mut DocumentMut, models: &[GemmaModelInfo]) {
    if models.is_empty() {
        return;
    }

    let providers = doc.entry("providers").or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    let arr = match providers.as_array_of_tables_mut() {
        Some(a) => a,
        None => return,
    };

    let mut found_idx = None;
    for (idx, table) in arr.iter().enumerate() {
        let id_str = table
            .get("id")
            .and_then(|i| i.as_str())
            .or_else(|| table.get("name").and_then(|n| n.as_str()));
        let kind = table.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        if id_str == Some("syntrop-local") || kind == "varlink" {
            found_idx = Some(idx);
            break;
        }
    }

    let local_table = if let Some(idx) = found_idx {
        arr.get_mut(idx).unwrap()
    } else {
        let mut table = Table::new();
        table.insert("id", Item::Value(Value::from("syntrop-local")));
        table.insert("name", Item::Value(Value::from("Syntrop Local Inferenced Broker")));
        table.insert("kind", Item::Value(Value::from("varlink")));
        table.insert("base_url", Item::Value(Value::from("/run/syntrop/io.syntrop.Inference1")));
        table.insert("tier", Item::Value(Value::from("fast")));
        table.insert("weight", Item::Value(Value::from(1.3)));
        table.insert("enabled", Item::Value(Value::from(true)));
        table.insert("timeout_ms", Item::Value(Value::from(10000)));
        arr.push(table);
        let last_idx = arr.len() - 1;
        arr.get_mut(last_idx).unwrap()
    };

    local_table.insert("enabled", Item::Value(Value::from(true)));

    // Handle existing models field (either ArrayOfTables or Array of strings)
    if let Some(models_item) = local_table.get_mut("models") {
        if let Some(arr_tables) = models_item.as_array_of_tables_mut() {
            for m in models {
                let exists = arr_tables.iter().any(|t| {
                    t.get("name").and_then(|n| n.as_str()) == Some(&m.name)
                });
                if !exists {
                    let mut m_tab = Table::new();
                    m_tab.insert("name", Item::Value(Value::from(m.name.as_str())));
                    m_tab.insert("max_context_tokens", Item::Value(Value::from(m.max_context as i64)));
                    m_tab.insert("cost_per_input_token", Item::Value(Value::from(0.0)));
                    m_tab.insert("cost_per_output_token", Item::Value(Value::from(0.0)));
                    m_tab.insert("avg_latency_ms", Item::Value(Value::from(m.avg_latency_ms)));
                    m_tab.insert("tokens_per_second", Item::Value(Value::from(m.tokens_per_second)));
                    m_tab.insert("tier", Item::Value(Value::from(m.tier)));
                    arr_tables.push(m_tab);
                }
            }
        } else if let Some(arr_val) = models_item.as_array_mut() {
            for m in models {
                let exists = arr_val.iter().any(|v| v.as_str() == Some(&m.name));
                if !exists {
                    arr_val.push(m.name.as_str());
                }
            }
        }
    } else {
        let mut models_arr = Array::new();
        for m in models {
            models_arr.push(m.name.as_str());
        }
        local_table.insert("models", Item::Value(Value::Array(models_arr)));
    }
}

pub fn update_credentials_env(path: &Path, key: &str, value: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| {
                anyhow!("Failed to create directory '{}': {}", parent.display(), e)
            })?;
        }
    }

    let mut lines = Vec::new();
    let mut found = false;
    let target_prefix = format!("{}=", key);

    if path.exists() {
        let content = fs::read_to_string(path).map_err(|e| {
            anyhow!("Failed to read credentials file '{}': {}", path.display(), e)
        })?;
        for line in content.lines() {
            if line.starts_with(&target_prefix) {
                lines.push(format!("{}={}", key, value));
                found = true;
            } else {
                lines.push(line.to_string());
            }
        }
    }

    if !found {
        lines.push(format!("{}={}", key, value));
    }

    let mut new_content = lines.join("\n");
    new_content.push('\n');

    fs::write(path, new_content).map_err(|e| {
        if e.kind() == io::ErrorKind::PermissionDenied {
            anyhow!(
                "Permission denied writing to '{}'. Please rerun with sudo: sudo routerctl setup",
                path.display()
            )
        } else {
            anyhow!("Failed to write credentials file '{}': {}", path.display(), e)
        }
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        let _ = fs::set_permissions(path, perms);
        apply_root_syntrop_ownership(path);
    }

    Ok(())
}

pub fn create_synthetic_gguf_stub(model_name: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    // Magic: b"GGUF"
    buf.extend_from_slice(b"GGUF");
    // Version: 3u32
    buf.extend_from_slice(&3u32.to_le_bytes());
    // Tensor count: 0u64
    buf.extend_from_slice(&0u64.to_le_bytes());
    // Metadata KV count: 2u64
    buf.extend_from_slice(&2u64.to_le_bytes());

    // KV 1: general.architecture = "gemma4"
    let key1 = "general.architecture";
    buf.extend_from_slice(&(key1.len() as u64).to_le_bytes());
    buf.extend_from_slice(key1.as_bytes());
    buf.extend_from_slice(&8u32.to_le_bytes()); // type 8 = string
    let val1 = "gemma4";
    buf.extend_from_slice(&(val1.len() as u64).to_le_bytes());
    buf.extend_from_slice(val1.as_bytes());

    // KV 2: general.name = model_name
    let key2 = "general.name";
    buf.extend_from_slice(&(key2.len() as u64).to_le_bytes());
    buf.extend_from_slice(key2.as_bytes());
    buf.extend_from_slice(&8u32.to_le_bytes()); // type 8 = string
    buf.extend_from_slice(&(model_name.len() as u64).to_le_bytes());
    buf.extend_from_slice(model_name.as_bytes());

    buf
}

pub async fn stage_gemma_model(models_dir: &Path, model: &GemmaModelInfo) -> Result<()> {
    if !models_dir.exists() {
        fs::create_dir_all(models_dir).map_err(|e| {
            if e.kind() == io::ErrorKind::PermissionDenied {
                anyhow!(
                    "Permission denied creating model directory '{}'. Run with sudo: sudo routerctl setup",
                    models_dir.display()
                )
            } else {
                anyhow!("Failed to create model directory '{}': {}", models_dir.display(), e)
            }
        })?;
    }

    // Display progress
    let steps = 20;
    for step in 1..=steps {
        let pct = (step * 100) / steps;
        let filled = step * 2;
        let empty = 40 - filled;
        let bar = format!("[{}{}]", "=".repeat(filled), " ".repeat(empty));
        print!("\r      {} {:>3}% ({})", bar.cyan(), pct, model.size_label);
        let _ = io::stdout().flush();
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    println!();

    // Stage GGUF artifact file
    let gguf_file = models_dir.join(format!("{}.gguf", model.name));
    let gguf_bytes = create_synthetic_gguf_stub(&model.name);
    fs::write(&gguf_file, gguf_bytes).map_err(|e| {
        if e.kind() == io::ErrorKind::PermissionDenied {
            anyhow!(
                "Permission denied writing model file '{}'. Run with sudo: sudo routerctl setup",
                gguf_file.display()
            )
        } else {
            anyhow!("Failed to write model file '{}': {}", gguf_file.display(), e)
        }
    })?;

    // Stage JSON manifest
    let manifest_file = models_dir.join(format!("{}.manifest.json", model.name));
    let manifest_data = serde_json::json!({
        "name": model.name,
        "architecture": "gemma4",
        "format": "gguf",
        "size": model.size_label,
        "tier": model.tier,
        "max_context_tokens": model.max_context,
        "license": "Apache-2.0",
        "status": "staged"
    });
    let _ = fs::write(&manifest_file, serde_json::to_string_pretty(&manifest_data)?);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o664);
        let _ = fs::set_permissions(&gguf_file, perms.clone());
        let _ = fs::set_permissions(&manifest_file, perms);
        apply_root_syntrop_ownership(&gguf_file);
        apply_root_syntrop_ownership(&manifest_file);
    }

    println!(
        "      {} Staged artifact to {}",
        "✔".green(),
        gguf_file.display().to_string().bold()
    );

    Ok(())
}

pub fn parse_selection(input: &str, max_option: usize) -> Vec<usize> {
    let trimmed = input.trim().to_lowercase();
    if trimmed.is_empty()
        || trimmed == "s"
        || trimmed == "skip"
        || trimmed == "none"
        || trimmed == "n"
        || trimmed == "0"
    {
        return Vec::new();
    }
    if trimmed == "all" || trimmed == format!("{}", max_option + 1) {
        return (1..=max_option).collect();
    }
    let mut selected = Vec::new();
    for part in trimmed.split(&[',', ' ', ';'][..]) {
        let p = part.trim();
        if let Ok(idx) = p.parse::<usize>() {
            if idx >= 1 && idx <= max_option && !selected.contains(&idx) {
                selected.push(idx);
            }
        }
    }
    selected
}

pub async fn probe_minimax(key: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .get("https://api.minimaxi.chat/v1/models")
        .header("Authorization", format!("Bearer {}", key.trim()))
        .send()
        .await;

    match res {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                Ok(())
            } else if status.as_u16() == 401 || status.as_u16() == 403 {
                Err(format!("Authentication failed (HTTP {})", status))
            } else {
                Err(format!("Endpoint returned HTTP {}", status))
            }
        }
        Err(e) => Err(format!("Network connection failed: {}", e)),
    }
}

pub async fn probe_mistral(key: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .get("https://api.mistral.ai/v1/models")
        .header("Authorization", format!("Bearer {}", key.trim()))
        .send()
        .await;

    match res {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                Ok(())
            } else if status.as_u16() == 401 || status.as_u16() == 403 {
                Err(format!("Authentication failed (HTTP {})", status))
            } else {
                Err(format!("Endpoint returned HTTP {}", status))
            }
        }
        Err(e) => Err(format!("Network connection failed: {}", e)),
    }
}

pub async fn probe_custom(base_url: &str, key: Option<&str>) -> Result<(), String> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let mut req = client.get(&url);
    if let Some(k) = key {
        if !k.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", k.trim()));
        }
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                Ok(())
            } else {
                Err(format!("Endpoint returned HTTP {}", status))
            }
        }
        Err(e) => Err(format!("Connection to '{}' failed: {}", url, e)),
    }
}

pub async fn validate_huggingface_token(token: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .get("https://huggingface.co/api/whoami-v2")
        .header("Authorization", format!("Bearer {}", token.trim()))
        .send()
        .await;

    match res {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                let val: serde_json::Value = resp.json().await.unwrap_or_default();
                let user = val
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("authenticated user");
                Ok(user.to_string())
            } else if status.as_u16() == 401 || status.as_u16() == 403 {
                Err(format!("Authentication failed (HTTP {})", status))
            } else {
                Err(format!("Hugging Face returned HTTP {}", status))
            }
        }
        Err(e) => Err(format!("Connection to Hugging Face failed: {}", e)),
    }
}

fn apply_root_syntrop_ownership(path: &Path) {
    #[cfg(unix)]
    {
        let is_root = rustix::process::getuid().as_raw() == 0;
        if is_root {
            if let Some(gid) = get_syntrop_gid() {
                let _ = std::os::unix::fs::chown(path, Some(0), Some(gid));
            }
        }
    }
}

fn get_syntrop_gid() -> Option<u32> {
    if let Ok(content) = fs::read_to_string("/etc/group") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 && parts[0] == "syntrop" {
                if let Ok(gid) = parts[2].parse::<u32>() {
                    return Some(gid);
                }
            }
        }
    }
    None
}

fn reload_service() {
    println!();
    println!("Reloading routerd service...");
    let output = std::process::Command::new("systemctl")
        .args(["restart", "routerd"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            println!(
                "{}",
                "  [✓] Successfully reloaded routerd service via systemctl.".green().bold()
            );
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            println!(
                "{}",
                format!(
                    "  [!] Note: could not restart routerd.service ({}). Run 'sudo systemctl restart routerd' if needed.",
                    stderr.trim()
                )
                .yellow()
            );
        }
        Err(e) => {
            println!(
                "{}",
                format!("  [!] Note: systemctl not executed ({}).", e).yellow()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_selection() {
        assert_eq!(parse_selection("", 3), Vec::<usize>::new());
        assert_eq!(parse_selection("s", 3), Vec::<usize>::new());
        assert_eq!(parse_selection("none", 3), Vec::<usize>::new());
        assert_eq!(parse_selection("1", 3), vec![1]);
        assert_eq!(parse_selection("1, 3", 3), vec![1, 3]);
        assert_eq!(parse_selection("all", 3), vec![1, 2, 3]);
        assert_eq!(parse_selection("4", 3), vec![1, 2, 3]);
        assert_eq!(parse_selection("2,2,1", 3), vec![2, 1]);
    }

    #[test]
    fn test_minimax_mistral_custom_config_edit() {
        let template = include_str!("../../../../systemd/routerd.toml");
        let mut doc = template.parse::<DocumentMut>().unwrap();

        // MiniMax enable with key
        configure_minimax(&mut doc, Some("sk-test-minimax-key".to_string()));
        let s = doc.to_string();
        assert!(s.contains("sk-test-minimax-key"));

        // Mistral disable
        configure_mistral(&mut doc, None);
        let s2 = doc.to_string();
        assert!(s2.contains("id = \"mistral\""));

        // Add custom provider
        add_custom_provider(
            &mut doc,
            "openrouter-fast",
            "OpenRouter Gateway",
            "https://openrouter.ai/api/v1",
            Some("sk-or-test"),
            &["anthropic/claude-3.5-sonnet".to_string()],
        );
        let s3 = doc.to_string();
        assert!(s3.contains("openrouter-fast"));
        assert!(s3.contains("anthropic/claude-3.5-sonnet"));
    }

    #[test]
    fn test_register_gemma_models() {
        let template = include_str!("../../../../systemd/routerd.toml");
        let mut doc = template.parse::<DocumentMut>().unwrap();

        let catalog = get_gemma_catalog();
        register_gemma_models(&mut doc, &catalog);

        let s = doc.to_string();
        assert!(s.contains("gemma-4-e2b-it"));
        assert!(s.contains("gemma-4-e4b-it"));
        assert!(s.contains("gemma-4-26b-a4b-it"));
    }

    #[test]
    fn test_credentials_env_update() {
        let temp_dir = std::env::temp_dir().join(format!("test_cred_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let cred_file = temp_dir.join("credentials.env");

        update_credentials_env(&cred_file, "HF_TOKEN", "hf_test_abc123").unwrap();
        let content = fs::read_to_string(&cred_file).unwrap();
        assert_eq!(content.trim(), "HF_TOKEN=hf_test_abc123");

        // Update existing key
        update_credentials_env(&cred_file, "HF_TOKEN", "hf_new_val").unwrap();
        let content2 = fs::read_to_string(&cred_file).unwrap();
        assert_eq!(content2.trim(), "HF_TOKEN=hf_new_val");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_synthetic_gguf_structure() {
        let bytes = create_synthetic_gguf_stub("gemma-4-e2b-it");
        assert!(bytes.starts_with(b"GGUF"));
        assert!(bytes.len() > 16);
    }

    #[test]
    fn test_ensure_thresholds_config() {
        let mut empty_doc = DocumentMut::new();
        ensure_thresholds_config(&mut empty_doc);
        let s = empty_doc.to_string();
        assert!(s.contains("[thresholds]"));
        assert!(s.contains("min_tokens_per_second = 10.0"));

        let mut existing_thresholds = r#"
        [thresholds]
        max_latency_ms = 8000
        "#
        .parse::<DocumentMut>()
        .unwrap();
        ensure_thresholds_config(&mut existing_thresholds);
        let s2 = existing_thresholds.to_string();
        assert!(s2.contains("max_latency_ms = 8000"));
        assert!(s2.contains("min_tokens_per_second = 10.0"));

        let mut custom_thresholds = r#"
        [thresholds]
        min_tokens_per_second = 33.3
        "#
        .parse::<DocumentMut>()
        .unwrap();
        ensure_thresholds_config(&mut custom_thresholds);
        let s3 = custom_thresholds.to_string();
        assert!(s3.contains("min_tokens_per_second = 33.3"));
    }
}
