//! rustytune server binary.
//!
//! Owns the ECU serial port (from Phase 2 on) and serves the browser UI:
//! the Vite-built frontend is embedded into this binary at compile time, so
//! a release build is a single self-contained executable. Binds loopback
//! only; opening it up to a LAN (in-car Pi deployment) is a later phase that
//! adds pairing/auth first.

use std::net::{IpAddr, SocketAddr};

use axum::{
    Json, Router,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use clap::Parser;
use rust_embed::RustEmbed;

/// Frontend build output (`npm run build` in web/). Run the web build before
/// `cargo build`, or you'll be serving an empty page.
#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct Assets;

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
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();

    let app = Router::new()
        .route("/api/health", get(health))
        .fallback(static_assets);

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

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": "rustytune",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Serve the embedded frontend; unknown extensionless paths fall back to
/// index.html so client-side routes survive a page reload.
async fn static_assets(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    let asset = Assets::get(path).or_else(|| {
        if path.contains('.') {
            None
        } else {
            Assets::get("index.html")
        }
    });

    match asset {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install ctrl-c handler");
}
