use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "routerctl")]
#[command(author = "Syntropd Authors")]
#[command(version = "0.3.0")]
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

    /// List all registered and available models
    Models,

    /// Simulate routing decision and score breakdown
    Route(RouteArgs),

    /// Benchmark router latency and Time-To-First-Token (TTFT)
    Test(TestArgs),

    /// Introspect Varlink interfaces and IDL specifications
    Info(InfoArgs),

    /// Interactive setup wizard for LLM providers, credentials, and Gemma 4 models
    Setup(SetupArgs),
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
pub struct InfoArgs {
    /// Specific interface to inspect (e.g. io.syntrop.Router1)
    pub interface: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct SetupArgs {
    /// Path to routerd configuration file
    #[arg(long, default_value = "/etc/syntrop/routerd.toml")]
    pub config: PathBuf,

    /// Path to credentials environment file
    #[arg(long, default_value = "/etc/syntrop/credentials.env")]
    pub credentials_path: PathBuf,

    /// Directory for downloaded/staged models
    #[arg(long, default_value = "/var/lib/models")]
    pub models_dir: PathBuf,

    /// Skip systemctl service reload
    #[arg(long)]
    pub no_reload: bool,
}
