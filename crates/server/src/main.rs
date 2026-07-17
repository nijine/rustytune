//! rustytune server binary.
//!
//! Owns the ECU serial port and serves the browser UI: the Vite-built
//! frontend is embedded into this binary at compile time, so a release
//! build is a single self-contained executable. Binds loopback only;
//! opening it up to a LAN (in-car Pi deployment) is a later phase that
//! adds pairing/auth first.

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(name = "rustytune", version, about = "Speeduino tuning server")]
struct Args {
    /// Port to listen on
    #[arg(long, default_value_t = 8642)]
    port: u16,

    /// Address to bind (keep loopback unless you know what you're doing)
    #[arg(long, default_value = "127.0.0.1")]
    bind: IpAddr,

    /// Don't open the browser after startup
    #[arg(long)]
    no_open: bool,

    /// ECU definition INI (default: the embedded Speeduino 202405-dev INI)
    #[arg(long)]
    ini: Option<PathBuf>,

    /// INI symbol to define, repeatable (e.g. --symbol CELSIUS)
    #[arg(long = "symbol")]
    symbols: Vec<String>,

    /// Directory for .msl datalogs
    #[arg(long, default_value = "logs")]
    log_dir: PathBuf,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();

    let ini_src = match &args.ini {
        Some(path) => std::fs::read_to_string(path)?,
        None => rustytune_server::EMBEDDED_INI.to_string(),
    };
    let symbols: HashSet<String> = args.symbols.iter().cloned().collect();
    let def = match ts_ini::parse_with_symbols(&ini_src, &symbols) {
        Ok(def) => def,
        Err(e) => {
            eprintln!("failed to parse ECU definition: {e}");
            std::process::exit(1);
        }
    };
    for warning in &def.warnings {
        tracing::warn!("ini: {warning}");
    }
    tracing::info!(
        "definition: {} ({} channels, {} gauges)",
        def.signature,
        def.output_channels.len(),
        def.gauges.len()
    );

    let state = rustytune_server::build_state_with_symbols(def, args.symbols.clone(), args.log_dir);
    let app = rustytune_server::app(state);

    let addr = SocketAddr::new(args.bind, args.port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let url = format!("http://{}", listener.local_addr()?);
    tracing::info!("listening on {url}");

    if !args.no_open
        && let Err(err) = open::that(&url)
    {
        tracing::warn!("could not open browser: {err}");
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install ctrl-c handler");
}
