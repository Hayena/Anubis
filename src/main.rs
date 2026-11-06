// Anubis: local forward proxy with adaptive routing.
//
// Entry point: CLI parse -> config load/validate -> component wiring ->
// graceful shutdown.

use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};

use anubis::api::server::{self, AppState};
use anubis::config::holder::ConfigHolder;
use anubis::config::loader::reload;
use anubis::config::models::ProxyType;
use anubis::health::store::{HealthStore, InMemoryHealthStore};
use anubis::metrics::collector::Metrics;
use anubis::proxy::listener::Proxy;
use std::net::{Ipv4Addr, SocketAddr};
use anubis::shared::error::RouterError;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn as_filter(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "anubis", version, about = "Local forward proxy with adaptive routing")]
struct CliArgs {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to HOCON config file
    #[arg(short, long, default_value = "proxy.conf")]
    config: String,

    /// Watch config file for changes (hot reload)
    #[arg(short, long)]
    watch: bool,

    /// Log level
    #[arg(short, long, value_enum, default_value_t = LogLevel::Info)]
    log_level: LogLevel,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the proxy server (default)
    Start,
    /// Configuration inspection
    #[command(subcommand)]
    Config(ConfigCommands),
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    /// Print the resolved configuration (validated)
    Print,
    /// Validate the configuration and report any errors
    Validate,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = CliArgs::parse();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(args.log_level.as_filter()))
        .init();

    let result = match &args.command {
        Some(Commands::Config(ConfigCommands::Print)) => run_config_print(&args.config),
        Some(Commands::Config(ConfigCommands::Validate)) => run_config_validate(&args.config),
        Some(Commands::Start) | None => run(&args).await,
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(target = "router", error = %e, "command failed");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: &CliArgs) -> Result<(), RouterError> {
    tracing::info!(
        target = "router",
        config = %args.config,
        watch = args.watch,
        "Starting Anubis"
    );

    let config = reload(&args.config)?;
    let holder = Arc::new(ConfigHolder::new(config));
    let metrics = Arc::new(Metrics::new());
    let health: Arc<dyn HealthStore> = Arc::new(InMemoryHealthStore::new());
    let state = Arc::new(AppState::new(
        Arc::clone(&holder),
        Arc::clone(&metrics),
        Arc::clone(&health),
        args.config.clone(),
    ));
    state.start_health_checks();

    let config = holder.current_config();

    let proxy_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, config.proxy_port));
    let proxy_listener = Proxy::bind(proxy_addr).await?;
    let proxy = Proxy::new(Arc::clone(&holder), Arc::clone(&metrics), health);
    tokio::spawn(async move {
        if let Err(e) = proxy.serve(proxy_listener).await {
            tracing::error!(target = "proxy", error = %e, "proxy listener failed");
        }
    });

    if args.watch {
        tokio::spawn(watch_config_file(args.config.clone(), Arc::clone(&state)));
    }

    let api_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, config.api_port));
    let api_listener = tokio::net::TcpListener::bind(api_addr).await?;
    tracing::info!(target = "router", bind = %api_addr, "REST API listening");

    axum::serve(api_listener, server::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!(target = "router", "shut down");
    Ok(())
}

async fn shutdown_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::error!(target = "router", error = %e, "failed to listen for ctrl-c");
        std::future::pending::<()>().await;
    }
    tracing::info!(target = "router", "shutdown signal received");
}

/// Reload the config when the file changes. A successful reload swaps the
/// routing state and connectors and restarts health checks; in-flight
/// tunnels are not interrupted. A failed reload keeps the old state.
async fn watch_config_file(path: String, state: Arc<AppState>) {
    use notify::{EventKind, RecursiveMode, Watcher, recommended_watcher};

    // Unbounded + non-blocking send keeps notify's thread from stalling, and
    // the async receiver never blocks a runtime worker.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher = match recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        let _ = tx.send(res);
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::error!(target = "config", error = %e, "failed to create config watcher");
            return;
        }
    };

    // Watch the parent directory: editors that save via rename/replace would
    // otherwise detach a watch placed on the file itself.
    let target = std::path::Path::new(&path);
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    if let Err(e) = watcher.watch(dir, RecursiveMode::NonRecursive) {
        tracing::error!(target = "config", error = %e, path = %path, "failed to watch config file");
        return;
    }
    tracing::info!(target = "config", path = %path, "config file watcher started");

    let file_name = target.file_name();
    while let Some(res) = rx.recv().await {
        match res {
            Ok(event)
                if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_))
                    && event.paths.iter().any(|p| p.file_name() == file_name) =>
            {
                // One save emits a burst of events; let it settle, then reload once.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                while rx.try_recv().is_ok() {}
                tracing::info!(target = "config", "config file changed; reloading");
                match state.reload() {
                    Ok(c) => tracing::info!(
                        target = "config",
                        proxies = c.proxies.len(),
                        rules = c.rules.len(),
                        "config reloaded"
                    ),
                    Err(e) => tracing::error!(
                        target = "config",
                        error = %e,
                        "config reload failed; keeping old config"
                    ),
                }
            }
            Ok(_) => {}
            Err(e) => tracing::error!(target = "config", error = %e, "config watch channel error"),
        }
    }
}

fn run_config_validate(config_path: &str) -> Result<(), RouterError> {
    reload(config_path)?;
    println!("Config is valid: {config_path}");
    Ok(())
}

/// Print the validated configuration as a compact tree.
fn run_config_print(config_path: &str) -> Result<(), RouterError> {
    let config = reload(config_path)?;

    println!("config: {config_path}");
    println!("listen: 127.0.0.1:{}", config.proxy_port);
    println!();
    println!("proxies:");
    for p in &config.proxies {
        let kind = match p.r#type {
            ProxyType::Http => "Http",
            ProxyType::Socks5 => "Socks5",
        };
        println!(
            "  - {}: {kind} -> {}:{}  [health: interval={}s, timeout={}s]",
            p.name,
            p.host,
            p.port,
            p.health.interval.as_secs(),
            p.health.timeout.as_secs()
        );
    }
    Ok(())
}
