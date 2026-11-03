// Adaptive Multi-Proxy Router

//

// Local forward proxy with adaptive routing.

// Entry point: CLI parse -> config load/validate -> component wiring ->

// graceful shutdown.



mod api;

mod config;

mod health;

mod metrics;

mod proxy;

mod routing;

mod shared;

mod upstream;



use std::net::ToSocketAddrs;


use std::sync::Arc;




use clap::{Parser, Subcommand};


use config::loader::{load, reload};

use config::models::ProxyType;

use health::store::InMemoryHealthStore;

use metrics::collector::Metrics;



use shared::error::RouterError;

use upstream::manager::UpstreamManager;

use upstream::traits::UpstreamConnector;



#[derive(Parser, Debug)]

#[command(name = "anubis", version = "0.1.0", about = "Local forward proxy with adaptive routing")]

struct CliArgs {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to HOCON config file

    #[arg(long, default_value = "proxy.conf")]

    config: String,



    /// Watch config file for changes (hot reload)

    #[arg(long)]

    watch: bool,



    /// Log level (off, error, warn, info, debug, trace)

    #[arg(long, default_value = "info")]

    log_level: String,

}



#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the proxy server (default)
    Start,
    /// Configuration inspection
    #[command(subcommand)]
    Config(ConfigCommands),
}
#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Print the resolved configuration (validated)
    Print,
    /// Validate the configuration and report any errors
    Validate,
}

async fn run_config_validate(config_path: &str) -> Result<(), RouterError> {
    // Use reload() which already validates the HOCON
    reload(config_path)?;
    println!("Config is valid: {}", config_path);
    Ok(())
}

#[tokio::main]
async fn main() {
    let args = CliArgs::parse();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(&args.log_level))
        .init();

    tracing::info!(
        target = "router",
        config = %args.config,
        watch = args.watch,
        log_level = %args.log_level,
        "Starting Adaptive Multi-Proxy Router"
    );

    match args.command {
        Some(Commands::Start) | None => {
            if let Err(e) = run(args).await {
                tracing::error!(target = "router", error = %e, "Startup failed");
                std::process::exit(1);
            }
        }
        Some(Commands::Config(ConfigCommands::Print)) => {
            if let Err(e) = run_config_print(&args.config) {
                tracing::error!(target = "router", error = %e, "config print failed");
                std::process::exit(1);
            }
            return;
        }
        Some(Commands::Config(ConfigCommands::Validate)) => {
            if let Err(e) = run_config_validate(&args.config).await {
                tracing::error!(target = "router", error = %e, "config validate failed");
                std::process::exit(1);
            }
            return;
        }
    }
}

async fn run(args: CliArgs) -> Result<(), RouterError> {

    // Milestone 1: Configuration loading.

    tracing::info!(target = "router", "Milestone 1: Configuration loading");

    let config = load(&args.config, args.watch).await?;



    // Milestone 2: Core route engine, held in a swappable `ConfigHolder`.

    let config_holder = api::server::ConfigHolder::new(config);

    let engine = config_holder.current_engine();

    let config = config_holder.current_config();



    // Demo routing decision.

    let test_dest = shared::types::Destination::new("dev.company.com", 443).map_err(|e| {

        RouterError::Config(shared::error::ConfigError::Validation(format!("test dest: {e}")))

    })?;

    let decision = engine.select(&test_dest);

    tracing::info!(target = "router", ?decision, "Route selection for destination");



    // Milestone 7: health store, metrics collector, upstream manager, API.

    let metrics = Arc::new(Metrics::new());

    let health = Arc::new(InMemoryHealthStore::new());

    let mut manager = UpstreamManager::new();



    // Register proxies and start health checkers.

    for proxy in &config.proxies {

        let connector = match proxy.r#type {

            ProxyType::Http => std::sync::Arc::new(

                upstream::http::HttpProxyConnector::new(&proxy.host, proxy.port),

            ) as std::sync::Arc<dyn UpstreamConnector>,

            ProxyType::Socks5 => std::sync::Arc::new(

                upstream::socks5::Socks5Upstream::new(&proxy.host, proxy.port),

            ) as std::sync::Arc<dyn UpstreamConnector>,

        };

        manager.register(proxy.name.clone(), connector);



        let checker = health::checker::HealthChecker::new(

            proxy.name.clone(),

            proxy.host.clone(),

            proxy.port,

            proxy.health.interval,

            proxy.health.timeout,

            health.clone(),

        );

        tokio::spawn(async move { checker.run().await });

        tracing::info!(target = "health", proxy = %proxy.name, "health checker started");

    }



    // Proxy groups have been removed; no group registration needed.

    // Milestone 3 (direct transport) + proxy request routing:

    // start the TCP forward proxy on the configured listen address.

    // Compute the listen address and proxy BEFORE moving config_holder into

    // the AppState (ConfigHolder is not Clone).

    let listen = config_holder.current_config().as_ref().listen.clone();

    let manager_arc = Arc::new(manager);

    let proxy = proxy::listener::Proxy::new(

        config_holder.current_engine(),

        manager_arc.clone(),

        Arc::clone(&metrics),

    );

    let listen_addr = (listen.host.as_str(), listen.port)

        .to_socket_addrs()

        .map_err(|e| RouterError::Io(std::io::Error::other(format!("resolve proxy listen: {e}"))))?

        .next()

        .ok_or_else(|| RouterError::Io(std::io::Error::other("empty proxy listen address")))?;

    tokio::spawn(async move {
        if let Err(e) = proxy.bind(listen_addr).await {
            tracing::error!(target = "proxy", error = %e, "proxy listener failed");
        }
    });
    tracing::info!(target = "proxy", bind = %listen_addr, "M3 direct transport proxy started");

    // Build shared API state with swappable config (Milestone 8).

    let state = Arc::new(api::server::AppState::new(
        config_holder,
        Arc::clone(&metrics),
        health.clone(),
        args.config.clone(),
    ));



    // Milestone 8: config file watcher (hot reload).

    if args.watch {

        let state_watch = state.clone();

        let path_watch = args.config.clone();

        tokio::spawn(watch_config_file(path_watch, state_watch));

    }



    // Milestone 7: start the REST API on the configured API host:API port.

    let api_addr = format!("{}:{}", config.api.host, config.api.port);

    let listener = tokio::net::TcpListener::bind(&api_addr).await.map_err(|e| {

        RouterError::Io(std::io::Error::other(format!("bind api: {e}")))

    })?;

    tracing::info!(target = "router", bind = %api_addr, "REST API listening");



    axum::serve(listener, api::server::router(state)).await.map_err(|e| {

        RouterError::Io(std::io::Error::other(format!("api serve: {e}")))

    })?;



    Ok(())

}



/// File watcher: reloads the config file when it changes (hot reload).

///

/// On a `Modify` event it re-reads the HOCON file, validates it, swaps the

/// `Arc<Config>`, and rebuilds the route engine — without dropping active

/// connections.

async fn watch_config_file(path: String, state: Arc<api::server::AppState>) {

    use notify::{recommended_watcher, RecursiveMode, Watcher};



    let (tx, rx) = std::sync::mpsc::sync_channel(1);

    let mut watcher = match recommended_watcher(move |res: Result<notify::Event, notify::Error>| {

        let _ = tx.send(res);

    }) {

        Ok(w) => w,

        Err(e) => {

            tracing::error!(target = "config", error = %e, "failed to create config watcher");

            return;

        }

    };



    if let Err(e) = watcher.watch(std::path::Path::new(&path), RecursiveMode::NonRecursive) {

        tracing::error!(target = "config", error = %e, path = %path, "failed to watch config file");

        return;

    }

    tracing::info!(target = "config", path = %path, "config file watcher started (M8 hot reload)");



    loop {

        let res = match rx.recv() {

            Ok(r) => r,

            Err(_) => break,

        };

        match res {

            Ok(event) => {

                if event.kind.is_modify() {

                    tracing::info!(target = "config", "config file modified; reloading (M8)");

                    handle_config_reload(&path, &state.config).await;

                }

            }

            Err(e) => tracing::error!(target = "config", error = %e, "config watch channel error"),

        }

    }

}



/// Swap the running config for the freshly parsed one and rebuild the route

/// engine. Does not touch the upstream manager (active connections preserved).

async fn handle_config_reload(path: &str, config_holder: &api::server::ConfigHolder) {
    match config_holder.reload(path) {
        Ok(new_config) => {
            tracing::info!(
                target = "config",
                "config reloaded; swapped Arc<Config> and rebuilt route engine (proxies: {}, rules: {})",
                new_config.proxies.len(),
                new_config.rules.len(),
            );
        }
        Err(e) => {
            tracing::error!(target = "config", error = %e, "config reload failed; keeping old config");
        }
    }
}

/// CLI: print the resolved configuration (validated) to stdout.
///
/// Reads and validates the HOCON file without starting listeners or
/// spawning tasks, then prints the internal model in a compact tree.
fn run_config_print(config_path: &str) -> Result<(), RouterError> {
    let config = reload(config_path)?;

    // Human-readable output — a compact tree, not raw JSON.
    fn fmt_duration(d: &std::time::Duration) -> String {
        format!("{}s", d.as_secs())
    }

    fn proxy_type_str(t: &config::models::ProxyType) -> &'static str {
        match t {
            config::models::ProxyType::Http => "Http",
            config::models::ProxyType::Socks5 => "Socks5",
        }
    }

    println!("config: {}", config_path);
    println!("listen: {}:{}", config.listen.host, config.listen.port);
    println!();
    println!("proxies:");
    for p in &config.proxies {
        let h = fmt_duration(&p.health.interval);
        let t = fmt_duration(&p.health.timeout);
        println!(
            "  - {}: {} -> {}:{}  [health: interval={}, timeout={}]",
            p.name, proxy_type_str(&p.r#type), p.host, p.port, h, t
        );
    }

    Ok(())
}
