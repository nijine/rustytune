//! rustytune server library: state construction and the axum router.
//! The binary (`main.rs`) is a thin CLI wrapper; integration tests drive
//! this same router in-process.

pub mod api;
pub mod comms;
pub mod definition;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::{
    Router,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rust_embed::RustEmbed;
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
pub const EMBEDDED_INI: &str = include_str!("../../../fixtures/speeduino202405_dev.ini");

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
    })
}

pub fn app(state: SharedState) -> Router {
    Router::new()
        .route("/api/health", get(api::health))
        .route("/api/ports", get(api::ports))
        .route("/api/status", get(api::status))
        .route("/api/definition", get(api::definition))
        .route("/api/connect", post(api::connect))
        .route("/api/disconnect", post(api::disconnect))
        .route("/api/log/start", post(api::log_start))
        .route("/api/log/stop", post(api::log_stop))
        .route("/api/logs", get(api::logs))
        .route("/api/logs/{name}", get(api::log_download))
        .route("/api/tune", get(api::tune_summary))
        .route("/api/tune/table/{id}", get(api::tune_table))
        .route("/api/tune/table/{id}/cells", post(api::tune_table_cells))
        .route("/api/tune/menus", get(api::tune_menus))
        .route("/api/tune/dialog/{name}", get(api::tune_dialog))
        .route("/api/tune/constants", get(api::tune_constants))
        .route("/api/tune/constant/{name}", post(api::tune_set_constant))
        .route("/api/tune/burn", post(api::tune_burn))
        .route("/api/msq", post(api::msq_upload))
        .route("/api/msq/diff", get(api::msq_diff))
        .route("/api/msq/apply", post(api::msq_apply))
        .route("/api/msq/save", get(api::msq_save))
        .route("/api/lock/release", post(api::lock_release))
        .route("/api/ws", get(api::ws))
        .fallback(static_assets)
        .with_state(state)
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
