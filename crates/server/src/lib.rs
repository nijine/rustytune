//! rustytune server library: state construction and the axum router.
//! The binary (`main.rs`) is a thin CLI wrapper; integration tests drive
//! this same router in-process.

pub mod admin;
pub mod api;
pub mod auth;
pub mod comms;
pub mod config;
pub mod definition;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::{future::Future, io};

use axum::{
    Router,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rust_embed::RustEmbed;
use std::time::Duration;
use tokio::sync::broadcast;

use api::{AppState, SharedState};
use definition::Defaults;

/// Frontend build output (`npm run build` in web/). Run the web build before
/// `cargo build`, or you'll be serving an empty page.
#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct Assets;

/// The Speeduino INI compiled into the binary as the out-of-box definition;
/// `--ini` overrides it for other firmware versions.
pub const EMBEDDED_INI: &str = include_str!("../../../fixtures/speeduino202501_7.ini");

pub fn build_state(def: ts_ini::IniDef, log_dir: PathBuf) -> SharedState {
    build_state_with_symbols(def, Vec::new(), log_dir)
}

pub fn build_state_with_symbols(
    def: ts_ini::IniDef,
    symbols: Vec<String>,
    log_dir: PathBuf,
) -> SharedState {
    let defaults = Arc::new(Defaults::from_ini(&def));
    let definition = Mutex::new(definition::definition_ui(&def, defaults.as_ref()));
    // Frame + status events; capacity covers a couple seconds of frames for
    // a briefly stalled client before it lags.
    let (events, _) = broadcast::channel(64);
    let def = Arc::new(def);
    Arc::new(AppState {
        def: def.clone(),
        defaults,
        definition,
        status: Arc::new(Mutex::new(comms::Status::default())),
        events,
        comms: Mutex::new(None),
        tune: Arc::new(Mutex::new(tune_model::Tune::new(def))),
        writer: Mutex::new(None),
        msq: Mutex::new(None),
        symbols,
        log_dir,
        runtime: Arc::new(config::RuntimeConfig::desktop()),
        auth: Arc::new(auth::AuthState::new(false, ".".into(), None)),
    })
}

pub fn build_appliance_state(
    def: ts_ini::IniDef,
    symbols: Vec<String>,
    runtime: config::RuntimeConfig,
) -> SharedState {
    build_state_with_runtime(def, symbols, runtime)
}

/// Build state with an explicit runtime profile. Native shells use this to
/// record their resolved data directory and ephemeral listener configuration.
pub fn build_state_with_runtime(
    def: ts_ini::IniDef,
    symbols: Vec<String>,
    runtime: config::RuntimeConfig,
) -> SharedState {
    let state = build_state_with_symbols(def, symbols, runtime.logging.directory.clone());
    // Construction is private to this module, so replace the immutable profile
    // before any clones are handed to request tasks.
    let mut owned = Arc::try_unwrap(state).ok().expect("fresh state");
    owned.auth = Arc::new(auth::AuthState::new(
        runtime.authentication.required,
        runtime.authentication.state_directory.clone(),
        runtime.authentication.master_pin.as_deref(),
    ));
    owned.runtime = Arc::new(runtime);
    Arc::new(owned)
}

pub fn app(state: SharedState) -> Router {
    let auth = state.auth.clone();
    Router::new()
        .route("/api/health", get(api::health))
        .route("/api/pair", post(auth::pair))
        .route("/api/session/logout", post(auth::logout))
        .route(
            "/api/appliance/config",
            get(api::appliance_config).put(api::appliance_config_put),
        )
        .route("/api/appliance/pairing", post(api::pairing_open))
        .route("/api/ports", get(api::ports))
        .route("/api/status", get(api::status))
        .route("/api/definition", get(api::definition))
        .route("/api/connect", post(api::connect))
        .route("/api/disconnect", post(api::disconnect))
        .route("/api/log/start", post(api::log_start))
        .route("/api/log/stop", post(api::log_stop))
        .route("/api/logs", get(api::logs))
        // Imports can be big (long TunerStudio sessions), hence the raised
        // body limit on this route.
        .route(
            "/api/logs/{name}",
            get(api::log_download)
                .post(api::log_import)
                .layer(axum::extract::DefaultBodyLimit::max(256 * 1024 * 1024)),
        )
        .route("/api/logs/{name}/data", get(api::log_data))
        .route("/api/tune", get(api::tune_summary))
        .route("/api/tune/table/{id}", get(api::tune_table))
        .route("/api/tune/table/{id}/cells", post(api::tune_table_cells))
        .route("/api/tune/table/{id}/axis", post(api::tune_table_axis))
        .route("/api/tune/curve/{id}", get(api::tune_curve))
        .route("/api/tune/curve/{id}/points", post(api::tune_curve_points))
        .route("/api/tune/menus", get(api::tune_menus))
        .route("/api/tune/dialog/{name}", get(api::tune_dialog))
        .route("/api/tune/constants", get(api::tune_constants))
        .route("/api/tune/constant/{name}", post(api::tune_set_constant))
        .route("/api/tune/burn", post(api::tune_burn))
        .route("/api/offline", post(api::offline_open))
        .route("/api/offline/close", post(api::offline_close))
        .route("/api/msq", post(api::msq_upload))
        .route("/api/msq/diff", get(api::msq_diff))
        .route("/api/msq/apply", post(api::msq_apply))
        .route("/api/msq/save", get(api::msq_save))
        .route("/api/lock/release", post(api::lock_release))
        .route("/api/ws", get(api::ws))
        .fallback(static_assets)
        .with_state(state)
        .layer(axum::middleware::from_fn_with_state(
            auth,
            auth::require_auth,
        ))
}

/// Appliance retry supervisor. It never makes the HTTP server unavailable.
pub fn spawn_auto_connect(state: SharedState) {
    if !state.runtime.ecu.auto_connect {
        return;
    }
    tokio::spawn(async move {
        let mut delay = Duration::from_secs(1);
        loop {
            if !state.status.lock().unwrap().connected {
                let cfg = &state.runtime.ecu;
                let req = api::ConnectReq {
                    port: cfg.device.clone(),
                    baud: cfg.baud,
                    mode: cfg.mode.clone(),
                    poll_ms: cfg.poll_ms,
                };
                let s = state.clone();
                let ok = tokio::task::spawn_blocking(move || api::do_connect(&s, req))
                    .await
                    .is_ok_and(|r| r.is_ok());
                if ok {
                    delay = Duration::from_secs(1);
                } else {
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(30));
                    continue;
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

/// Serve the application on an already-bound listener and shut down all ECU
/// communication once the HTTP server stops. Both the CLI and native desktop
/// shell use this entry point so serial and datalog cleanup cannot diverge.
pub async fn serve_with_shutdown<F>(
    listener: tokio::net::TcpListener,
    state: SharedState,
    shutdown: F,
) -> io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let result = axum::serve(listener, app(state.clone()))
        .with_graceful_shutdown(shutdown)
        .await;
    shutdown_comms(&state);
    result
}

/// Stop and join the communication thread. Taking the handle makes repeated
/// cleanup requests harmless and guarantees an active datalog is flushed once.
pub fn shutdown_comms(state: &SharedState) {
    let handle = state.comms.lock().unwrap().take();
    if let Some(handle) = handle {
        let _ = handle.cmd_tx.send(comms::Cmd::Shutdown);
        let _ = handle.join.join();
    }
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
