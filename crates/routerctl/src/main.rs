mod cli;
mod client;
mod commands;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands, ProvidersCommands};
use client::RouterctlClient;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();
    let client = RouterctlClient::new(&args.socket, &args.http_url);

    match args.command {
        Commands::Status => {
            commands::status::run_status(&client, args.json).await?;
        }
        Commands::Providers(p_args) => match p_args.command {
            ProvidersCommands::List => {
                commands::providers::run_list(&client, args.json).await?;
            }
            ProvidersCommands::Test { provider_id } => {
                commands::providers::run_test(&client, provider_id, args.json).await?;
            }
        },
        Commands::Models => {
            commands::models::run_models(&client, args.json).await?;
        }
        Commands::Route(r_args) => {
            commands::route::run_route(&client, r_args, args.json).await?;
        }
        Commands::Test(t_args) => {
            commands::test::run_benchmark(&client, t_args, args.json).await?;
        }
        Commands::Info(i_args) => {
            commands::info::run_info(&client, i_args, args.json).await?;
        }
    }

    Ok(())
}
