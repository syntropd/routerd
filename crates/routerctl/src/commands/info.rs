use crate::cli::InfoArgs;
use crate::client::RouterctlClient;
use anyhow::Result;
use colored::Colorize;

pub async fn run_info(
    client: &RouterctlClient,
    args: InfoArgs,
    json_output: bool,
) -> Result<()> {
    if let Some(iface) = args.interface {
        let idl = client.get_interface_description(&iface).await?;
        if json_output {
            let obj = serde_json::json!({
                "interface": iface,
                "idl": idl
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        } else {
            println!("{}", format!("=== Interface: {} ===", iface).bold().cyan());
            println!("{}", idl);
        }
    } else {
        let info = client.get_info().await?;
        if json_output {
            println!("{}", serde_json::to_string_pretty(&info)?);
        } else {
            println!("{}", "=== Varlink Service Info ===".bold().cyan());
            let vendor = info.get("vendor").and_then(|v| v.as_str()).unwrap_or("");
            let product = info.get("product").and_then(|v| v.as_str()).unwrap_or("");
            let version = info.get("version").and_then(|v| v.as_str()).unwrap_or("");
            let url = info.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let ifaces = info
                .get("interfaces")
                .and_then(|i| i.as_array())
                .cloned()
                .unwrap_or_default();

            println!("  {:<16} {}", "Vendor:".bold(), vendor);
            println!("  {:<16} {}", "Product:".bold(), product);
            println!("  {:<16} {}", "Version:".bold(), version);
            println!("  {:<16} {}", "URL:".bold(), url);
            println!("  {:<16}", "Interfaces:".bold());
            for iface in ifaces {
                if let Some(name) = iface.as_str() {
                    println!("    - {}", name.green());
                }
            }
        }
    }

    Ok(())
}
