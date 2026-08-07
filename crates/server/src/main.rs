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
use rustytune_server::config::{Profile, RuntimeConfig};

#[derive(Parser)]
#[command(name = "rustytune", version, about = "Speeduino tuning server")]
struct Args {
    /// Runtime deployment profile (desktop remains the default)
    #[arg(long, value_enum, default_value_t=Profile::Desktop)]
    profile: Profile,
    /// TOML configuration file (appliance default: /etc/rustytune/rustytune.toml)
    #[arg(long)]
    config: Option<PathBuf>,
    /// Port to listen on
    #[arg(long)]
    port: Option<u16>,

    /// Address to bind (keep loopback unless you know what you're doing)
    #[arg(long)]
    bind: Option<IpAddr>,

    /// Don't open the browser after startup
    #[arg(long)]
    no_open: bool,

    /// ECU definition INI (default: the embedded Speeduino 202501 INI)
    #[arg(long)]
    ini: Option<PathBuf>,

    /// INI symbol to define, repeatable (e.g. --symbol CELSIUS)
    #[arg(long = "symbol")]
    symbols: Vec<String>,

    /// Directory for .msl datalogs
    #[arg(long)]
    log_dir: Option<PathBuf>,
    /// Permit an unauthenticated non-loopback appliance listener (development only)
    #[arg(long)]
    allow_unsafe_no_auth: bool,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();
    let base = match args.profile {
        Profile::Desktop => RuntimeConfig::desktop(),
        Profile::Appliance => RuntimeConfig::appliance(),
    };
    let config_path = args.config.clone().or_else(|| {
        (args.profile == Profile::Appliance).then(|| PathBuf::from("/etc/rustytune/rustytune.toml"))
    });
    let mut runtime = match config_path.as_deref() {
        Some(p) => RuntimeConfig::load(p, base).map_err(std::io::Error::other)?,
        None => base,
    };
    if let Some(v) = args.bind {
        runtime.server.bind = v;
    }
    if let Some(v) = args.port {
        runtime.server.port = v;
    }
    if args.no_open {
        runtime.server.open_browser = false;
    }
    if let Some(v) = &args.log_dir {
        runtime.logging.directory = v.clone();
    }
    if let Some(v) = &args.ini {
        runtime.ecu.ini = Some(v.clone());
    }
    runtime.validate().map_err(std::io::Error::other)?;
    if args.profile == Profile::Appliance
        && !runtime.server.bind.is_loopback()
        && !runtime.authentication.required
        && !args.allow_unsafe_no_auth
    {
        return Err(std::io::Error::other(
            "refusing unauthenticated non-loopback appliance listener; enable authentication or pass --allow-unsafe-no-auth",
        ));
    }

    let ini_src = match &runtime.ecu.ini {
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

    let state = if args.profile == Profile::Appliance {
        rustytune_server::build_appliance_state(def, args.symbols.clone(), runtime.clone())
    } else {
        rustytune_server::build_state_with_symbols(
            def,
            args.symbols.clone(),
            runtime.logging.directory.clone(),
        )
    };
    rustytune_server::spawn_auto_connect(state.clone());
    if let Some(path) = runtime.server.admin_socket.clone() {
        rustytune_server::admin::spawn(state.clone(), path)?;
    }

    let addr = SocketAddr::new(runtime.server.bind, runtime.server.port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let url = format!("http://{}", listener.local_addr()?);
    tracing::info!("listening on {url}");

    if runtime.server.open_browser
        && let Err(err) = open::that(&url)
    {
        tracing::warn!("could not open browser: {err}");
    }

    rustytune_server::serve_with_shutdown(listener, state, shutdown_signal()).await?;
    tracing::info!("bye");
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler");
        tokio::select! {
            r = tokio::signal::ctrl_c() => r.expect("failed to install ctrl-c handler"),
            _ = terminate.recv() => tracing::info!("SIGTERM received, shutting down"),
            _ = quit_key() => tracing::info!("q pressed, shutting down"),
        }
    }
    #[cfg(not(unix))]
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
                Ok(1) => {}  // any other key: keep listening
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
