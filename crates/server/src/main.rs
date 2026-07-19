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
    let app = rustytune_server::app(state.clone());

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

    // Stop the comms thread so an in-flight .msl log is flushed and the
    // serial port closes before the process exits.
    let handle = state.comms.lock().unwrap().take();
    if let Some(handle) = handle {
        let _ = handle.cmd_tx.send(rustytune_server::comms::Cmd::Shutdown);
        let _ = handle.join.join();
    }
    tracing::info!("bye");
    Ok(())
}

async fn shutdown_signal() {
    tokio::select! {
        r = tokio::signal::ctrl_c() => r.expect("failed to install ctrl-c handler"),
        _ = quit_key() => tracing::info!("q pressed, shutting down"),
    }
}

/// Resolves when `q` is pressed on the controlling terminal. When stdin is
/// not a tty (piped, CI, service) this never resolves and Ctrl+C remains
/// the only trigger.
async fn quit_key() {
    let Some(_raw) = RawStdin::new() else {
        return std::future::pending().await;
    };
    tracing::info!("press q to quit");
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 1];
        loop {
            match std::io::stdin().read(&mut buf) {
                Ok(1) if matches!(buf[0], b'q' | b'Q') => {
                    let _ = tx.send(());
                    return;
                }
                Ok(1) => {} // any other key: keep listening
                _ => return, // EOF or error: fall back to Ctrl+C
            }
        }
    });
    if rx.await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Puts stdin into non-canonical, no-echo mode so single keypresses arrive
/// without Enter; restores the original settings on drop. Ctrl+C keeps
/// working (ISIG is left on).
struct RawStdin {
    saved: libc::termios,
}

impl RawStdin {
    fn new() -> Option<RawStdin> {
        unsafe {
            if libc::isatty(libc::STDIN_FILENO) != 1 {
                return None;
            }
            let mut saved: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut saved) != 0 {
                return None;
            }
            let mut raw = saved;
            raw.c_lflag &= !(libc::ICANON | libc::ECHO);
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) != 0 {
                return None;
            }
            Some(RawStdin { saved })
        }
    }
}

impl Drop for RawStdin {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.saved);
        }
    }
}
