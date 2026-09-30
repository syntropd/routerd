use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "routerctl")]
#[command(author = "Syntropd Authors")]
#[command(version)]
#[command(about = "Operator control CLI for syntrop-routerd")]
pub struct Cli {
    #[arg(short, long, default_value = "/run/syntrop/io.syntrop.Router1", global = true)]
    pub socket: PathBuf,

    #[arg(long, default_value = "http://127.0.0.1:32768", global = true)]
    pub http_url: String,

    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Show daemon status, memory RSS, active requests, and PSI metrics
    Status,

    /// Inspect and test LLM providers
    Providers(ProvidersArgs),

    /// List connected provider models
    Models,

    /// Show or set the default model (pinned first when eligible)
    Default(DefaultArgs),

    /// Simulate routing decision and score breakdown
    Route(RouteArgs),

    /// Benchmark router latency and Time-To-First-Token (TTFT)
    Test(TestArgs),

    /// Introspect Varlink interfaces and IDL specifications
    Info(InfoArgs),

    /// Verify providers live and enable only what answers
    Setup(SetupArgs),

    /// Ask a question and print the model's reply
    Ask(AskArgs),
}

#[derive(Args, Debug)]
pub struct ProvidersArgs {
    #[command(subcommand)]
    pub command: ProvidersCommands,
}

#[derive(Subcommand, Debug)]
pub enum ProvidersCommands {
    /// List configured providers and health status
    List,

    /// Test connectivity and latency of provider(s)
    Test {
        /// Specific provider ID to test (tests all if omitted)
        provider_id: Option<String>,
    },
}

#[derive(Args, Debug)]
pub struct RouteArgs {
    /// Target model name (or 'router:fast', 'router:hard', 'auto')
    #[arg(default_value = "router:fast")]
    pub model: String,

    /// Difficulty tier ('fast' or 'hard')
    #[arg(short, long)]
    pub tier: Option<String>,

    /// Estimated context tokens (prompt + output)
    #[arg(long, default_value_t = 1024)]
    pub tokens: usize,

    /// Simulate streaming request
    #[arg(long)]
    pub stream: bool,
}

#[derive(Args, Debug)]
pub struct TestArgs {
    /// Model to test
    #[arg(short, long, default_value = "router:fast")]
    pub model: String,

    /// Benchmark prompt to send
    #[arg(short, long, default_value = "Respond with one short sentence acknowledging this benchmark test.")]
    pub prompt: String,

    /// Difficulty tier ('fast' or 'hard')
    #[arg(short, long)]
    pub tier: Option<String>,
}

#[derive(Args, Debug)]
pub struct DefaultArgs {
    /// Model to pin as default (omit to show the current default)
    pub model: Option<String>,

    /// Path to routerd configuration file
    #[arg(long, default_value = "/etc/syntrop/routerd.toml")]
    pub config: PathBuf,

    /// Skip systemctl service reload
    #[arg(long)]
    pub no_reload: bool,
}

#[derive(Args, Debug)]
pub struct InfoArgs {
    /// Specific interface to inspect (e.g. io.syntrop.Router1)
    pub interface: Option<String>,
}

#[derive(Args, Debug)]
pub struct AskArgs {
    /// Model or alias (router:auto, router:fast, router:hard, or a model name)
    #[arg(short, long, default_value = "router:auto")]
    pub model: String,

    /// Reasoning effort tier (none, low, medium, high, max; defaults to 0 tokens on CPU / tight memory, 1,024 on GPU with healthy VRAM)
    #[arg(short = 'e', long = "effort", alias = "reasoning-effort")]
    pub effort: Option<String>,

    /// Max completion tokens
    #[arg(long, default_value_t = 256)]
    pub max_tokens: usize,

    /// Question words (joined with spaces)
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    pub prompt: Vec<String>,
}

#[derive(Args, Debug, Clone)]
pub struct SetupArgs {
    /// Path to routerd configuration file
    #[arg(long, default_value = "/etc/syntrop/routerd.toml")]
    pub config: PathBuf,

    /// Directory for downloaded/staged models
    #[arg(long, default_value = "/var/lib/models")]
    pub models_dir: PathBuf,

    /// Skip systemctl service reload
    #[arg(long)]
    pub no_reload: bool,

    /// Non-interactive: skip prompts, auto-pick the first live model
    #[arg(long)]
    pub auto: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_status_and_route() {
        let cli = Cli::try_parse_from(["routerctl", "status"]).unwrap();
        assert!(matches!(cli.command, Commands::Status));
        assert!(!cli.json);
        let cli =
            Cli::try_parse_from(["routerctl", "--json", "route", "router:hard", "--tokens", "5"]).unwrap();
        assert!(cli.json);
        match cli.command {
            Commands::Route(r) => {
                assert_eq!(r.model, "router:hard");
                assert_eq!(r.tokens, 5);
            }
            _ => panic!("expected route"),
        }
    }
}
