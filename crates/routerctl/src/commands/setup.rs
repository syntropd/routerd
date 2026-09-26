use crate::cli::SetupArgs;
use anyhow::{anyhow, Result};
use colored::Colorize;
use futures::future::join_all;
use routerd_core::credentials::resolve_credential;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value};

/// Timeout for a single provider probe during setup.
const PROBE_TIMEOUT_SECS: u64 = 5;
/// Files smaller than this are stubs, not models.
const MIN_REAL_MODEL_BYTES: u64 = 1_000_000;
/// Well-known local Ollama endpoint (OpenAI-compatible path).
const LOCAL_OLLAMA_URL: &str = "http://127.0.0.1:11434/v1";
/// Socket proving the local inference broker is alive.
const INFERENCED_SOCKET: &str = "/run/syntrop/io.syntrop.Inference1";
const MINIMAX_BASE_URL: &str = "https://api.minimaxi.chat/v1";
const MISTRAL_BASE_URL: &str = "https://api.mistral.ai/v1";

// Setup registers only servers and model files it verifies live.
// There is deliberately no model-download step: bring Ollama or GGUF files,
// re-run setup, and they get picked up.

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
    println!("{}", "routerctl setup — ping everything, enable only what answers".bold());
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

    println!("{}", "keys".cyan());
    run_key_step(
        &mut doc,
        &mut report,
        "minimax",
        MINIMAX_BASE_URL,
        configure_minimax,
    )
    .await;
    run_key_step(
        &mut doc,
        &mut report,
        "mistral",
        MISTRAL_BASE_URL,
        configure_mistral,
    )
    .await;
    println!();

    println!("{}", "custom".cyan());
    run_custom_step(&mut doc, &mut report).await;
    println!();

    println!("{}", "default".cyan());
    pick_default_model(&mut doc);
    println!();

    save_config(&args.config, &doc)?;
    println!("saved {}", args.config.display().to_string().bold());

    if !args.no_reload {
        reload_service();
    }

    print_summary(&report);
    Ok(())
}

/// One hosted-key step: prompt, ping /models, store only a live answer.
async fn run_key_step(
    doc: &mut DocumentMut,
    report: &mut Vec<(String, String)>,
    id: &str,
    base_url: &str,
    configure: fn(&mut DocumentMut, Option<String>, Option<Vec<String>>),
) {
    let key = prompt_line(&format!("{} key [skip]: ", id));
    if key.is_empty() {
        configure(doc, None, None);
        report.push((id.to_string(), "OFF · skipped".to_string()));
        println!("  {} OFF · skipped", id.bold());
        return;
    }
    print!("  {} pinging... ", id.bold());
    let _ = io::stdout().flush();
    match fetch_model_ids(base_url, Some(key.as_str())).await {
        Ok(ids) => {
            println!("{}", format!("LIVE · {} model(s)", ids.len()).green().bold());
            configure(doc, Some(key), Some(ids.clone()));
            report.push((id.to_string(), format!("LIVE · {} model(s)", ids.len())));
        }
        Err(e) => {
            println!("{} {}", "OFF ·".red().bold(), e);
            configure(doc, None, None);
            report.push((id.to_string(), format!("OFF · {}", e)));
        }
    }
}

/// One custom-provider step: prompt, ping, store only a live answer (or an
/// explicit save-anyway, which stays OFF).
async fn run_custom_step(doc: &mut DocumentMut, report: &mut Vec<(String, String)>) {
    let add_custom = prompt_line("add a custom provider (vLLM, OpenRouter...)? [y/N]: ");
    if !(add_custom.eq_ignore_ascii_case("y") || add_custom.eq_ignore_ascii_case("yes")) {
        return;
    }
    let p_id = prompt_line("  id [custom-provider]: ");
    let p_id = if p_id.is_empty() { "custom-provider".to_string() } else { p_id };
    let p_name = prompt_line("  name [same as id]: ");
    let p_name = if p_name.is_empty() { p_id.clone() } else { p_name };
    let base_url = prompt_line("  base URL [https://api.openai.com/v1]: ");
    let base_url = if base_url.is_empty() {
        "https://api.openai.com/v1".to_string()
    } else {
        base_url.trim_end_matches('/').to_string()
    };
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

pub(crate) fn load_or_init_config(path: &Path) -> Result<DocumentMut> {
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

pub(crate) fn save_config(path: &Path, doc: &DocumentMut) -> Result<()> {
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

pub fn configure_minimax(
    doc: &mut DocumentMut,
    api_key: Option<String>,
    models: Option<Vec<String>>,
) {
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
                    table.insert("base_url", Item::Value(Value::from(MINIMAX_BASE_URL)));
                    table.insert("api_key", Item::Value(Value::from(key.as_str())));
                    if let Some(ids) = &models {
                        set_models_array(table, ids);
                    }
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
        if let Some(key) = &api_key {
            let mut table = Table::new();
            table.insert("id", Item::Value(Value::from("minimax")));
            table.insert("name", Item::Value(Value::from("MiniMax AI Cloud")));
            table.insert("kind", Item::Value(Value::from("minimax")));
            table.insert("base_url", Item::Value(Value::from(MINIMAX_BASE_URL)));
            table.insert("api_key", Item::Value(Value::from(key.as_str())));
            table.insert("enabled", Item::Value(Value::from(true)));
            table.insert("tier", Item::Value(Value::from("hard")));
            table.insert("weight", Item::Value(Value::from(1.0)));
            table.insert("timeout_ms", Item::Value(Value::from(30000)));
            set_models_array(&mut table, &models.unwrap_or_default());
            arr.push(table);
        }
    }
}

pub fn configure_mistral(
    doc: &mut DocumentMut,
    api_key: Option<String>,
    models: Option<Vec<String>>,
) {
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
                    table.insert("base_url", Item::Value(Value::from(MISTRAL_BASE_URL)));
                    table.insert("api_key", Item::Value(Value::from(key.as_str())));
                    if let Some(ids) = &models {
                        set_models_array(table, ids);
                    }
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
        if let Some(key) = &api_key {
            let mut table = Table::new();
            table.insert("id", Item::Value(Value::from("mistral")));
            table.insert("name", Item::Value(Value::from("Mistral AI Platform")));
            table.insert("kind", Item::Value(Value::from("openai")));
            table.insert("base_url", Item::Value(Value::from(MISTRAL_BASE_URL)));
            table.insert("api_key", Item::Value(Value::from(key.as_str())));
            table.insert("enabled", Item::Value(Value::from(true)));
            table.insert("tier", Item::Value(Value::from("hard")));
            table.insert("weight", Item::Value(Value::from(1.0)));
            table.insert("timeout_ms", Item::Value(Value::from(25000)));
            set_models_array(&mut table, &models.unwrap_or_default());
            arr.push(table);
        }
    }
}

/// Insert or update a provider by id. Re-running setup never duplicates.
pub fn add_custom_provider(
    doc: &mut DocumentMut,
    id: &str,
    name: &str,
    kind: &str,
    base_url: &str,
    api_key: Option<&str>,
    models: &[String],
    enabled: bool,
) {
    let providers = doc.entry("providers").or_insert(Item::ArrayOfTables(ArrayOfTables::new()));
    let arr = match providers.as_array_of_tables_mut() {
        Some(a) => a,
        None => return,
    };

    for table in arr.iter_mut() {
        let id_str = table
            .get("id")
            .and_then(|i| i.as_str())
            .or_else(|| table.get("name").and_then(|n| n.as_str()));
        if id_str == Some(id) {
            table.insert("name", Item::Value(Value::from(if name.is_empty() { id } else { name })));
            table.insert("kind", Item::Value(Value::from(kind)));
            table.insert("base_url", Item::Value(Value::from(base_url)));
            if let Some(key) = api_key {
                if !key.is_empty() {
                    table.insert("api_key", Item::Value(Value::from(key)));
                }
            }
            set_models_array(table, models);
            table.insert("enabled", Item::Value(Value::from(enabled)));
            return;
        }
    }

    let mut table = Table::new();
    table.insert("id", Item::Value(Value::from(id)));
    table.insert("name", Item::Value(Value::from(if name.is_empty() { id } else { name })));
    table.insert("kind", Item::Value(Value::from(kind)));
    table.insert("base_url", Item::Value(Value::from(base_url)));
    if let Some(key) = api_key {
        if !key.is_empty() {
            table.insert("api_key", Item::Value(Value::from(key)));
        }
    }
    table.insert("tier", Item::Value(Value::from("fast")));
    table.insert("weight", Item::Value(Value::from(1.0)));
    table.insert("enabled", Item::Value(Value::from(enabled)));
    table.insert("timeout_ms", Item::Value(Value::from(30000)));
    set_models_array(&mut table, models);
    arr.push(table);
}

/// Register real on-disk model files under the local varlink provider.
/// Merges with whatever is already there; never duplicates.
pub fn upsert_local_models(doc: &mut DocumentMut, names: &[String], enabled: bool) {
    if names.is_empty() {
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
        table.insert("base_url", Item::Value(Value::from(INFERENCED_SOCKET)));
        table.insert("tier", Item::Value(Value::from("fast")));
        table.insert("weight", Item::Value(Value::from(1.3)));
        table.insert("enabled", Item::Value(Value::from(enabled)));
        table.insert("timeout_ms", Item::Value(Value::from(10000)));
        arr.push(table);
        let last_idx = arr.len() - 1;
        arr.get_mut(last_idx).unwrap()
    };

    local_table.insert("enabled", Item::Value(Value::from(enabled)));

    // Merge names into whatever models shape is already there.
    if let Some(models_item) = local_table.get_mut("models") {
        if let Some(arr_tables) = models_item.as_array_of_tables_mut() {
            for name in names {
                let exists = arr_tables.iter().any(|t| {
                    t.get("name").and_then(|n| n.as_str()) == Some(name.as_str())
                });
                if !exists {
                    let mut m_tab = Table::new();
                    m_tab.insert("name", Item::Value(Value::from(name.as_str())));
                    arr_tables.push(m_tab);
                }
            }
        } else if let Some(arr_val) = models_item.as_array_mut() {
            for name in names {
                let exists = arr_val.iter().any(|v| v.as_str() == Some(name.as_str()));
                if !exists {
                    arr_val.push(name.as_str());
                }
            }
        }
    } else {
        set_models_array(local_table, names);
    }
}

/// Overwrite a provider table's models list with plain verified names.
fn set_models_array(table: &mut Table, models: &[String]) {
    let mut arr = Array::new();
    for m in models {
        arr.push(m.as_str());
    }
    table.insert("models", Item::Value(Value::Array(arr)));
}

/// OpenAI shape: {"data": [{"id": ...}]}.
pub fn parse_openai_model_ids(val: &serde_json::Value) -> Vec<String> {
    val.get("data")
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    m.get("id")
                        .and_then(|i| i.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Ollama shape: {"models": [{"name": ...}]}.
pub fn parse_ollama_model_ids(val: &serde_json::Value) -> Vec<String> {
    val.get("models")
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    m.get("name")
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// GET {base_url}/models and return the live model IDs.
/// Understands OpenAI ({data:[{id}]}) and Ollama ({models:[{name}]}) shapes.
pub async fn fetch_model_ids(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .build()
        .map_err(|e| e.to_string())?;
    let base = base_url.trim_end_matches('/');
    let mut req = client.get(format!("{}/models", base));
    if let Some(k) = api_key {
        if !k.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", k.trim()));
        }
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;
    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(format!("Key rejected (HTTP {})", status));
    }
    if status.as_u16() == 404 && !base.contains("/v1") {
        // Native Ollama answers on /api/tags instead.
        return fetch_ollama_tags(base, &client).await;
    }
    if !status.is_success() {
        return Err(format!("Endpoint returned HTTP {}", status));
    }
    let val: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad response: {}", e))?;
    let mut ids = parse_openai_model_ids(&val);
    if ids.is_empty() {
        ids = parse_ollama_model_ids(&val);
    }
    if ids.is_empty() {
        return Err("Endpoint answered but listed no models".to_string());
    }
    Ok(ids)
}

async fn fetch_ollama_tags(
    base: &str,
    client: &reqwest::Client,
) -> Result<Vec<String>, String> {
    let resp = client
        .get(format!("{}/api/tags", base))
        .send()
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("Endpoint returned HTTP {}", resp.status()));
    }
    let val: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Bad response: {}", e))?;
    let ids = parse_ollama_model_ids(&val);
    if ids.is_empty() {
        return Err("Endpoint answered but listed no models".to_string());
    }
    Ok(ids)
}

struct AuditEntry {
    idx: usize,
    id: String,
    kind: String,
    base_url: String,
    key_spec: Option<String>,
}

/// Ping every enabled provider already in the document, in parallel.
/// Dead entries get switched off; live ones get their model lists refreshed
/// from the answers. Never prompts.
async fn audit_existing_providers(doc: &mut DocumentMut, report: &mut Vec<(String, String)>) {
    let mut entries: Vec<AuditEntry> = Vec::new();
    if let Some(arr) = doc.get("providers").and_then(|p| p.as_array_of_tables()) {
        for (idx, table) in arr.iter().enumerate() {
            if table.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                continue;
            }
            let id = table
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            let kind = table
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("openai")
                .to_string();
            let base_url = table
                .get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let key_spec = table
                .get("api_key")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            entries.push(AuditEntry {
                idx,
                id,
                kind,
                base_url,
                key_spec,
            });
        }
    }
    if entries.is_empty() {
        println!("  nothing configured yet");
        return;
    }
    let probes = entries.iter().map(|e| async move {
        if e.kind == "varlink" {
            if Path::new(&e.base_url).exists() {
                (e.idx, e.id.clone(), Ok(Vec::new()))
            } else {
                (
                    e.idx,
                    e.id.clone(),
                    Err("local socket missing".to_string()),
                )
            }
        } else {
            let key = e
                .key_spec
                .as_deref()
                .and_then(|spec| resolve_credential(Some(spec), &e.id).unwrap_or(None));
            match fetch_model_ids(&e.base_url, key.as_deref()).await {
                Ok(ids) => (e.idx, e.id.clone(), Ok(ids)),
                Err(err) => (e.idx, e.id.clone(), Err(err)),
            }
        }
    });
    for (idx, id, result) in join_all(probes).await {
        let arr = match doc
            .get_mut("providers")
            .and_then(|p| p.as_array_of_tables_mut())
        {
            Some(a) => a,
            None => continue,
        };
        let Some(table) = arr.get_mut(idx) else {
            continue;
        };
        match result {
            Ok(ids) => {
                if !ids.is_empty() {
                    set_models_array(table, &ids);
                }
                table.insert("enabled", Item::Value(Value::from(true)));
                let line = if ids.is_empty() {
                    "LIVE · local".to_string()
                } else {
                    format!("LIVE · {} model(s)", ids.len())
                };
                println!("  {:<16} {}", id.bold(), line);
                report.push((id, line));
            }
            Err(e) => {
                table.insert("enabled", Item::Value(Value::from(false)));
                println!("  {:<16} {} {}", id.bold(), "OFF ·".red().bold(), e);
                report.push((id, format!("OFF · {}", e)));
            }
        }
    }
}

/// Find what's usable on this machine: a local Ollama server and real
/// on-disk model files. Registers only what exists. Never prompts.
async fn autodetect_local(
    doc: &mut DocumentMut,
    models_dir: &Path,
    report: &mut Vec<(String, String)>,
) {
    let mut found_any = false;
    match fetch_model_ids(LOCAL_OLLAMA_URL, None).await {
        Ok(ids) => {
            add_custom_provider(
                doc,
                "local-ollama",
                "Local Ollama",
                "ollama",
                LOCAL_OLLAMA_URL,
                None,
                &ids,
                true,
            );
            println!("  {:<16} {}", "ollama".bold(), format!("LIVE · {} model(s)", ids.len()));
            report.push((
                "local-ollama".to_string(),
                format!("LIVE · {} model(s)", ids.len()),
            ));
            found_any = true;
        }
        Err(_) => {
            println!("  {:<16} OFF · nothing on port 11434", "ollama".bold());
        }
    }
    let mut files: Vec<String> = Vec::new();
    if let Ok(rd) = fs::read_dir(models_dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            let is_gguf = path.extension().and_then(|x| x.to_str()) == Some("gguf");
            let is_real = entry
                .metadata()
                .map(|m| m.len() > MIN_REAL_MODEL_BYTES)
                .unwrap_or(false);
            if is_gguf && is_real {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    files.push(stem.to_string());
                }
            }
        }
    }
    if files.is_empty() {
        println!("  {:<16} none in {}", "model files".bold(), models_dir.display());
    } else {
        let alive = Path::new(INFERENCED_SOCKET).exists();
        upsert_local_models(doc, &files, alive);
        if alive {
            println!("  {:<16} LIVE · {} file(s)", "model files".bold(), files.len());
            report.push((
                "syntrop-local".to_string(),
                format!("LIVE · {} file(s)", files.len()),
            ));
        } else {
            println!("  {:<16} OFF · broker not running", "model files".bold());
            report.push((
                "syntrop-local".to_string(),
                "OFF · broker not running".to_string(),
            ));
        }
        found_any = true;
    }
    if !found_any {
        println!("  hint: install Ollama, `ollama pull qwen2.5-coder:7b`, re-run setup");
    }
}

/// Enabled providers' models as (provider id, model name) pairs.
pub fn live_models(doc: &DocumentMut) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(arr) = doc.get("providers").and_then(|p| p.as_array_of_tables()) else {
        return out;
    };
    for table in arr.iter() {
        if table.get("enabled").and_then(|v| v.as_bool()) != Some(true) {
            continue;
        }
        let id = table
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        let Some(models) = table.get("models") else { continue };
        if let Some(names) = models.as_array() {
            for v in names.iter().filter_map(|v| v.as_str()) {
                out.push((id.clone(), v.to_string()));
            }
        } else if let Some(tables) = models.as_array_of_tables() {
            for t in tables.iter().filter_map(|t| t.get("name").and_then(|n| n.as_str())) {
                out.push((id.clone(), t.to_string()));
            }
        }
    }
    out
}

/// Accept a 1-based number, an exact model name, or skip/empty.
pub fn parse_default_pick(input: &str, options: &[(String, String)]) -> Option<String> {
    let answer = input.trim();
    if answer.is_empty() || answer.eq_ignore_ascii_case("skip") || answer.eq_ignore_ascii_case("n") {
        return None;
    }
    if let Ok(n) = answer.parse::<usize>() {
        if n >= 1 && n <= options.len() {
            return Some(options[n - 1].1.clone());
        }
        return None;
    }
    options
        .iter()
        .find(|(_, name)| name.eq_ignore_ascii_case(answer))
        .map(|(_, name)| name.clone())
}

/// Write a tier's default model, creating tables as needed.
pub fn set_tier_default(doc: &mut DocumentMut, tier: &str, model: &str) {
    if doc.get("tiers").is_none() {
        doc["tiers"] = Item::Table(Table::new());
    }
    let tiers = &mut doc["tiers"];
    if tiers.get(tier).is_none() {
        tiers[tier] = Item::Table(Table::new());
    }
    tiers[tier]["default_model"] = Item::Value(Value::from(model));
}

fn pick_default_model(doc: &mut DocumentMut) {
    let options = live_models(doc);
    if options.is_empty() {
        println!("  no verified models; skipping");
        return;
    }
    for (i, (prov, name)) in options.iter().enumerate() {
        println!("  {}) {} · {}", i + 1, prov, name.bold());
    }
    let answer = prompt_line(&format!("default [1-{}, name, skip]: ", options.len()));
    match parse_default_pick(&answer, &options) {
        Some(model) => {
            set_tier_default(doc, "fast", &model);
            set_tier_default(doc, "hard", &model);
            println!("  default → {} (fast + hard)", model.bold());
        }
        None => println!("  no default; scoring decides"),
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
    println!("models: routerctl models · try it: routerctl test");
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

pub(crate) fn reload_service() {
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
    fn test_parse_model_shapes() {
        let openai: serde_json::Value = serde_json::json!({
            "data": [{"id": "a"}, {"id": "b"}, {"nope": 1}]
        });
        assert_eq!(
            parse_openai_model_ids(&openai),
            vec!["a".to_string(), "b".to_string()]
        );
        let ollama: serde_json::Value = serde_json::json!({
            "models": [{"name": "x"}, {"name": "y"}]
        });
        assert_eq!(
            parse_ollama_model_ids(&ollama),
            vec!["x".to_string(), "y".to_string()]
        );
        let empty: serde_json::Value = serde_json::json!({});
        assert!(parse_openai_model_ids(&empty).is_empty());
        assert!(parse_ollama_model_ids(&empty).is_empty());
    }

    #[test]
    fn test_upsert_local_models() {
        let mut doc = DocumentMut::new();
        doc["providers"] = Item::ArrayOfTables(ArrayOfTables::new());
        upsert_local_models(&mut doc, &["m1".to_string()], true);
        upsert_local_models(&mut doc, &["m1".to_string(), "m2".to_string()], true);
        let s = doc.to_string();
        assert_eq!(s.matches("\"m1\"").count(), 1);
        assert!(s.contains("\"m2\""));
        assert!(s.contains("id = \"syntrop-local\""));
    }

    #[test]
    fn test_minimax_mistral_custom_config_edit() {
        let template = include_str!("../../../../systemd/routerd.toml");
        let mut doc = template.parse::<DocumentMut>().unwrap();

        // MiniMax enable with key and verified models
        configure_minimax(
            &mut doc,
            Some("unit-test-key-minimax".to_string()),
            Some(vec!["live-model-a".to_string()]),
        );
        let s = doc.to_string();
        assert!(s.contains("unit-test-key-minimax"));
        assert!(s.contains("live-model-a"));

        // Mistral disable
        configure_mistral(&mut doc, None, None);
        let s2 = doc.to_string();
        let doc2: DocumentMut = s2.parse().unwrap();
        let mistral = doc2["providers"]
            .as_array_of_tables()
            .unwrap()
            .iter()
            .find(|t| t.get("id").and_then(|v| v.as_str()) == Some("mistral"))
            .unwrap();
        assert_eq!(
            mistral.get("enabled").and_then(|v| v.as_bool()),
            Some(false)
        );

        // Add custom provider twice: must upsert, never duplicate
        for _ in 0..2 {
            add_custom_provider(
                &mut doc,
                "openrouter-fast",
                "OpenRouter Gateway",
                "openai",
                "https://openrouter.ai/api/v1",
                Some("sk-or-test"),
                &["anthropic/claude-3.5-sonnet".to_string()],
                true,
            );
        }
        let s3 = doc.to_string();
        assert_eq!(s3.matches("id = \"openrouter-fast\"").count(), 1);
        assert!(s3.contains("anthropic/claude-3.5-sonnet"));
    }

    #[test]
    fn test_default_pick_parsing() {
        let options = vec![
            ("prov-a".to_string(), "model-a".to_string()),
            ("prov-b".to_string(), "model-b".to_string()),
        ];
        assert_eq!(parse_default_pick("1", &options), Some("model-a".to_string()));
        assert_eq!(parse_default_pick("2", &options), Some("model-b".to_string()));
        assert_eq!(parse_default_pick("model-b", &options), Some("model-b".to_string()));
        assert_eq!(parse_default_pick("MODEL-A", &options), Some("model-a".to_string()));
        assert_eq!(parse_default_pick("", &options), None);
        assert_eq!(parse_default_pick("skip", &options), None);
        assert_eq!(parse_default_pick("0", &options), None);
        assert_eq!(parse_default_pick("9", &options), None);
        assert_eq!(parse_default_pick("nope", &options), None);
    }

    #[test]
    fn test_live_models_and_tier_default() {
        let mut doc = r#"
[[providers]]
id = "on"
enabled = true
models = ["a", "b"]

[[providers]]
id = "detailed"
enabled = true

[[providers.models]]
name = "d"

[[providers]]
id = "off"
enabled = false
models = ["c"]
"#
        .parse::<DocumentMut>()
        .unwrap();
        let live = live_models(&doc);
        assert_eq!(
            live,
            vec![
                ("on".to_string(), "a".to_string()),
                ("on".to_string(), "b".to_string()),
                ("detailed".to_string(), "d".to_string()),
            ]
        );

        set_tier_default(&mut doc, "fast", "b");
        set_tier_default(&mut doc, "hard", "b");
        assert_eq!(
            doc["tiers"]["fast"]["default_model"].as_str(),
            Some("b")
        );
        assert_eq!(
            doc["tiers"]["hard"]["default_model"].as_str(),
            Some("b")
        );
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
