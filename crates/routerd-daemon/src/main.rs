mod activation;
mod gateway;
mod notify;
mod rss;
mod server;
mod varlink;

use activation::check_and_adopt_sockets;
use anyhow::{Context, Result};
use clap::Parser;
use gateway::create_gateway_router;
use routerd_core::{RouterConfig, RouterEngine};
use rss::MemoryStats;
use server::{bind_standalone_tcp, bind_standalone_unix, serve_tcp_gateway, serve_unix_gateway};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::sync::broadcast;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Parser, Debug)]
#[command(name = "routerd")]
#[command(author = "Syntropd Authors")]
#[command(version = "0.3.0")]
#[command(about = "Intelligent model router, difficulty-tier evaluation, and wire protocol gateway daemon for syntropd")]
pub struct Cli {
    #[arg(short, long, default_value = "/etc/syntrop/routerd.toml")]
    config: PathBuf,

    #[arg(long, default_value = "127.0.0.1:32768")]
    listen_tcp: String,

    #[arg(long, default_value = "/run/syntrop/router.sock")]
    listen_unix: PathBuf,

    #[arg(long, default_value = "/run/syntrop/io.syntrop.Router1")]
    varlink_socket: PathBuf,

    #[arg(long, default_value = "/run/syntrop/io.syntrop.Inference1")]
    inferenced_socket: PathBuf,

    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let env_filter = if cli.verbose {
        EnvFilter::new("debug,routerd=debug,routerd_core=debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,routerd=info"))
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting routerd v0.3.0");

    // Load configuration or generate fallback defaults
    let config = if cli.config.exists() {
        info!("Loading configuration from {:?}", cli.config);
        RouterConfig::load_from_file(&cli.config)
            .with_context(|| format!("Failed to load configuration file {:?}", cli.config))?
    } else {
        info!("Configuration file {:?} not found, using embedded defaults", cli.config);
        let default_toml = include_str!("../../../systemd/routerd.toml");
        RouterConfig::load_from_str(default_toml).unwrap_or_default()
    };

    let engine = Arc::new(RouterEngine::new(config));

    // Adopt systemd socket descriptors if socket activated
    let mut activated = check_and_adopt_sockets()?;

    // 1. Prepare Varlink Listener
    let varlink_listener = match activated.varlink.take() {
        Some(listener) => {
            info!("Using systemd-activated Varlink listener");
            listener
        }
        None => {
            let path = &cli.varlink_socket;
            info!("Binding standalone Varlink socket at {:?}", path);
            match varlink::bind_or_create_listener(path) {
                Ok(l) => l,
                Err(e) => {
                    let fallback_path = PathBuf::from("/tmp/syntrop_io.syntrop.Router1");
                    warn!("Failed to bind {:?}: {}. Falling back to {:?}", path, e, fallback_path);
                    varlink::bind_or_create_listener(&fallback_path)?
                }
            }
        }
    };

    // 2. Prepare Unix Gateway Listener
    let unix_gateway_listener = match activated.unix_gateway.take() {
        Some(l) => {
            info!("Using systemd-activated Unix gateway listener");
            Some(l)
        }
        None => {
            let path = &cli.listen_unix;
            info!("Binding standalone Unix gateway at {:?}", path);
            match bind_standalone_unix(path) {
                Ok(l) => Some(l),
                Err(e) => {
                    let fallback = PathBuf::from("/tmp/syntrop_router.sock");
                    warn!("Failed to bind {:?}: {}. Falling back to {:?}", path, e, fallback);
                    bind_standalone_unix(&fallback).ok()
                }
            }
        }
    };

    // 3. Prepare TCP Gateway Listeners
    let mut tcp_listeners = activated.tcp_gateways;
    if tcp_listeners.is_empty() {
        info!("Binding standalone TCP gateway at {}", cli.listen_tcp);
        match bind_standalone_tcp(&cli.listen_tcp).await {
            Ok(l) => tcp_listeners.push(l),
            Err(e) => warn!("Failed to bind TCP {}: {}", cli.listen_tcp, e),
        }

        // Try dual-stack IPv6 if listening on localhost
        if cli.listen_tcp.starts_with("127.0.0.1:") {
            let v6_addr = format!("[::1]:{}", cli.listen_tcp.split(':').nth(1).unwrap_or("32768"));
            if let Ok(l) = bind_standalone_tcp(&v6_addr).await {
                info!("Binding standalone IPv6 TCP gateway at {}", v6_addr);
                tcp_listeners.push(l);
            }
        }
    }

    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    // Spawn Varlink Server
    let varlink_engine = engine.clone();
    tokio::spawn(async move {
        varlink::run_varlink_listener(varlink_listener, varlink_engine).await;
    });

    // Spawn Axum Reverse Proxy on Unix domain socket
    if let Some(unix_l) = unix_gateway_listener {
        let app = create_gateway_router(engine.clone());
        let mut rx = shutdown_tx.subscribe();
        tokio::spawn(async move {
            let shutdown = async move {
                let _ = rx.recv().await;
            };
            if let Err(e) = serve_unix_gateway(unix_l, app, shutdown).await {
                error!("Unix gateway server terminated: {}", e);
            }
        });
    }

    // Spawn Axum Reverse Proxy on TCP listeners
    for tcp_l in tcp_listeners {
        let app = create_gateway_router(engine.clone());
        let mut rx = shutdown_tx.subscribe();
        tokio::spawn(async move {
            let shutdown = async move {
                let _ = rx.recv().await;
            };
            if let Err(e) = serve_tcp_gateway(tcp_l, app, shutdown).await {
                error!("TCP gateway server terminated: {}", e);
            }
        });
    }

    // Spawn RSS Monitor task (<15MB target)
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            let mem = MemoryStats::read_current();
            info!(
                "routerd RSS: {:.2} MB (page size: {}B)",
                mem.rss_mb(),
                mem.page_size
            );
        }
    });

    // Spawn Watchdog task
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            interval.tick().await;
            notify::notify_systemd_watchdog();
        }
    });

    // Notify systemd READY
    notify::notify_systemd_ready();

    // Wait for termination signal
    match signal::ctrl_c().await {
        Ok(()) => {
            info!("Received shutdown signal, terminating cleanly...");
        }
        Err(err) => {
            error!("Error listening for shutdown signal: {}", err);
        }
    }

    notify::notify_systemd_stopping();
    let _ = shutdown_tx.send(());

    // Give in-flight requests a short moment to finish
    tokio::time::sleep(Duration::from_millis(200)).await;
    info!("routerd shutdown complete.");
    Ok(())
}
