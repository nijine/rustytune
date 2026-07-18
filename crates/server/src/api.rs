//! REST + WebSocket handlers. All tune/telemetry logic stays server-side;
//! the frontend is a thin renderer of these responses.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    Json,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use ecu_proto::{Config, Mode, SerialTransport, Session};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use ts_ini::{DialogItem, MenuItem, Value};
use tune_model::Tune;

use crate::comms::{self, Cmd, CommsCtx, CommsHandle, Status};
use crate::definition::{Defaults, DefinitionUi};

pub struct AppState {
    pub def: Arc<ts_ini::IniDef>,
    pub defaults: Arc<Defaults>,
    /// Gauge/indicator UI with bounds resolved; refreshed when PcVariables
    /// (gauge limits) change.
    pub definition: Mutex<DefinitionUi>,
    pub status: Arc<Mutex<Status>>,
    pub events: broadcast::Sender<String>,
    pub comms: Mutex<Option<CommsHandle>>,
    pub tune: Arc<Mutex<Tune>>,
    /// Client id holding the tuning write lock (other clients read-only).
    pub writer: Mutex<Option<String>>,
    /// Uploaded .msq reference file: (filename, parsed).
    pub msq: Mutex<Option<(String, tune_model::MsqFile)>>,
    /// Symbols the INI was parsed with (recorded in saved .msq settings).
    pub symbols: Vec<String>,
    pub log_dir: PathBuf,
}

pub type SharedState = Arc<AppState>;

/// JSON error body with an HTTP status. Kept small (clippy result_large_err);
/// becomes a full `Response` only at the handler edge.
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

fn err(code: StatusCode, msg: impl Into<String>) -> ApiError {
    ApiError(code, msg.into())
}

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": "rustytune",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Serialize)]
pub struct PortInfo {
    pub path: String,
    /// Looks like a USB serial adapter (sorted first in the picker).
    pub usb: bool,
}

fn is_serial_name(name: &str) -> bool {
    // macOS callout devices and Linux USB/ACM/UART serial nodes.
    name.starts_with("cu.")
        || name.starts_with("ttyUSB")
        || name.starts_with("ttyACM")
        || name.starts_with("ttyAMA")
}

fn looks_usb(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "usbserial",
        "usbmodem",
        "wchusb",
        "slab",
        "ttyusb",
        "ttyacm",
    ]
    .iter()
    .any(|p| lower.contains(p))
}

pub async fn ports() -> Json<Vec<PortInfo>> {
    let mut out: Vec<PortInfo> = std::fs::read_dir("/dev")
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    is_serial_name(&name).then(|| PortInfo {
                        path: format!("/dev/{name}"),
                        usb: looks_usb(&name),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.usb.cmp(&a.usb).then(a.path.cmp(&b.path)));
    // Hardware-free bench (tools/bench.sh): fake-ECU pty symlinks.
    if let Ok(rd) = std::fs::read_dir("/tmp") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("rustytune-sim") {
                out.push(PortInfo {
                    path: format!("/tmp/{name}"),
                    usb: false,
                });
            }
        }
    }
    Json(out)
}

pub async fn status(State(state): State<SharedState>) -> Json<Status> {
    Json(state.status.lock().unwrap().clone())
}

pub async fn definition(State(state): State<SharedState>) -> Json<DefinitionUi> {
    Json(state.definition.lock().unwrap().clone())
}

/// Re-resolve gauge bounds (expressions over PcVariables like `{rpmhigh}`)
/// and push the fresh definition to every connected client.
fn refresh_definition(state: &AppState, tune: &Tune) {
    let ui = crate::definition::definition_ui(
        &state.def,
        &crate::definition::PcOverlay {
            tune,
            defaults: &state.defaults,
        },
    );
    let msg = serde_json::json!({ "type": "definition", "definition": &ui }).to_string();
    *state.definition.lock().unwrap() = ui;
    let _ = state.events.send(msg);
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectReq {
    pub port: String,
    #[serde(default = "default_baud")]
    pub baud: u32,
    /// "primary" (USB, CRC envelope) or "secondary" (SER3, raw).
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_poll_ms")]
    pub poll_ms: u64,
}

fn default_baud() -> u32 {
    115_200
}
fn default_mode() -> String {
    "primary".into()
}
fn default_poll_ms() -> u64 {
    50
}

pub async fn connect(State(state): State<SharedState>, Json(req): Json<ConnectReq>) -> Response {
    let result = tokio::task::spawn_blocking(move || do_connect(&state, req)).await;
    match result {
        Ok(Ok(status)) => Json(status).into_response(),
        Ok(Err(resp)) => resp.into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn do_connect(state: &SharedState, req: ConnectReq) -> Result<Status, ApiError> {
    let mode = match req.mode.as_str() {
        "primary" => Mode::Primary,
        "secondary" => Mode::Secondary,
        other => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                format!("unknown mode `{other}` (want primary|secondary)"),
            ));
        }
    };

    // Connecting replaces the tune; don't silently drop offline edits.
    if state.status.lock().unwrap().offline && state.tune.lock().unwrap().any_dirty() {
        return Err(err(
            StatusCode::CONFLICT,
            "offline tune has unsaved changes — save it as .msq or close the offline session first",
        ));
    }

    let mut comms = state.comms.lock().unwrap();
    if let Some(handle) = comms.take() {
        if state.status.lock().unwrap().connected {
            *comms = Some(handle);
            return Err(err(StatusCode::CONFLICT, "already connected"));
        }
        // The thread exited on its own (device unplugged); reap it.
        let _ = handle.cmd_tx.send(Cmd::Shutdown);
        let _ = handle.join.join();
    }

    let transport = SerialTransport::open(&req.port, req.baud)
        .map_err(|e| err(StatusCode::BAD_REQUEST, format!("open {}: {e}", req.port)))?;
    let config = Config::new(
        mode,
        &state.def.och_get_command,
        state.def.och_block_size as u16,
    );
    let session = Session::new(transport, config)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    {
        let mut status = state.status.lock().unwrap();
        *status = Status {
            connected: true,
            port: Some(req.port.clone()),
            mode: Some(req.mode.clone()),
            baud: Some(req.baud),
            ..Status::default()
        };
    }
    // Fresh tune snapshot and lock for this connection; app-side
    // PcVariables (gauge limits) carry over.
    {
        let mut tune = state.tune.lock().unwrap();
        let mut fresh = Tune::new(state.def.clone());
        fresh.adopt_pc_values(&tune);
        *tune = fresh;
    }
    *state.writer.lock().unwrap() = None;

    let pages = match mode {
        Mode::Primary => Some(
            comms::page_commands(&state.def)
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
        ),
        Mode::Secondary => None,
    };
    let ctx = CommsCtx {
        def: state.def.clone(),
        defaults: state.defaults.clone(),
        status: state.status.clone(),
        events: state.events.clone(),
        poll_interval: Duration::from_millis(req.poll_ms.clamp(20, 1000)),
        tune: state.tune.clone(),
        pages,
    };
    let delay = state
        .def
        .header
        .delay_after_port_open
        .map(|ms| Duration::from_millis(ms as u64));
    *comms = Some(comms::spawn(session, ctx, delay));
    drop(comms);

    let status = state.status.lock().unwrap().clone();
    let _ = state.events.send(comms::status_message(&status));
    Ok(status)
}

pub async fn disconnect(State(state): State<SharedState>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let handle = state.comms.lock().unwrap().take();
        match handle {
            Some(handle) => {
                let _ = handle.cmd_tx.send(Cmd::Shutdown);
                let _ = handle.join.join();
                Ok(state.status.lock().unwrap().clone())
            }
            None => Err(err(StatusCode::CONFLICT, "not connected")),
        }
    })
    .await;
    match result {
        Ok(Ok(status)) => Json(status).into_response(),
        Ok(Err(resp)) => resp.into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Send a command to the comms thread and wait briefly for its reply.
fn comms_roundtrip<R: Send + 'static>(
    state: &SharedState,
    make_cmd: impl FnOnce(mpsc::Sender<R>) -> Cmd,
) -> Result<R, ApiError> {
    let comms = state.comms.lock().unwrap();
    let Some(handle) = comms.as_ref() else {
        return Err(err(StatusCode::CONFLICT, "not connected"));
    };
    let (reply_tx, reply_rx) = mpsc::channel();
    handle
        .cmd_tx
        .send(make_cmd(reply_tx))
        .map_err(|_| err(StatusCode::CONFLICT, "comms thread gone"))?;
    drop(comms);
    reply_rx.recv_timeout(Duration::from_secs(3)).map_err(|_| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "comms thread not responding",
        )
    })
}

pub async fn log_start(State(state): State<SharedState>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let name = format!(
            "rustytune_{}.msl",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );
        let path = state.log_dir.join(name);
        comms_roundtrip(&state, |reply| Cmd::StartLog { path, reply })
    })
    .await;
    match result {
        Ok(Ok(Ok(path))) => Json(serde_json::json!({ "path": path })).into_response(),
        Ok(Ok(Err(msg))) => err(StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        Ok(Err(resp)) => resp.into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LogFile {
    name: String,
    size: u64,
    /// Local mtime, "YYYY-MM-DD HH:MM:SS".
    modified: String,
    /// Currently being written by the running log session.
    active: bool,
}

/// The datalog directory listing: where the .msl files live and what's
/// there, newest first.
pub async fn logs(State(state): State<SharedState>) -> Response {
    let active = state
        .status
        .lock()
        .unwrap()
        .log
        .as_ref()
        .map(|l| l.path.clone());
    let dir = state.log_dir.clone();
    let listing = tokio::task::spawn_blocking(move || {
        let mut files: Vec<LogFile> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        if !name.ends_with(".msl") {
                            return None;
                        }
                        let meta = e.metadata().ok()?;
                        let modified = meta
                            .modified()
                            .ok()
                            .map(|t| {
                                chrono::DateTime::<chrono::Local>::from(t)
                                    .format("%Y-%m-%d %H:%M:%S")
                                    .to_string()
                            })
                            .unwrap_or_default();
                        let active = active.as_deref().is_some_and(|p| {
                            std::path::Path::new(p).file_name() == Some(e.file_name().as_ref())
                        });
                        Some(LogFile {
                            name,
                            size: meta.len(),
                            modified,
                            active,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.name.cmp(&b.name)));
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        serde_json::json!({ "dir": dir.to_string_lossy(), "files": files })
    })
    .await;
    match listing {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn log_download(State(state): State<SharedState>, Path(name): Path<String>) -> Response {
    // Names come from our own listing; anything path-like is rejected.
    if name.contains('/') || name.contains('\\') || name.contains("..") || !name.ends_with(".msl") {
        return err(StatusCode::BAD_REQUEST, "invalid log name").into_response();
    }
    let path = state.log_dir.join(&name);
    let read = tokio::task::spawn_blocking(move || std::fs::read(path)).await;
    match read {
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        Ok(Ok(bytes)) => (
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{name}\""),
                ),
            ],
            bytes,
        )
            .into_response(),
        Ok(Err(e)) => err(StatusCode::NOT_FOUND, format!("{name}: {e}")).into_response(),
    }
}

pub async fn log_stop(State(state): State<SharedState>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        comms_roundtrip(&state, |reply| Cmd::StopLog { reply })
    })
    .await;
    match result {
        Ok(Ok(Some(summary))) => Json(summary).into_response(),
        Ok(Ok(None)) => err(StatusCode::CONFLICT, "not logging").into_response(),
        Ok(Err(resp)) => resp.into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn ws(ws: WebSocketUpgrade, State(state): State<SharedState>) -> Response {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: SharedState) {
    let mut rx = state.events.subscribe();

    // Current status first, so a late-joining client renders immediately.
    let snapshot = comms::status_message(&state.status.lock().unwrap());
    if socket.send(Message::Text(snapshot.into())).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            event = rx.recv() => match event {
                Ok(text) => {
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                // Slow client skipped some frames; keep going with fresh ones.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(_)) => {} // clients only listen; ignore pings/noise
                _ => break,
            },
        }
    }
}

// ----- tune API -------------------------------------------------------------

fn client_id(headers: &HeaderMap) -> String {
    headers
        .get("x-client-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous")
        .to_string()
}

/// First writer takes the lock; everyone else is read-only until release.
fn acquire_writer(state: &SharedState, headers: &HeaderMap) -> Result<(), ApiError> {
    let id = client_id(headers);
    let mut writer = state.writer.lock().unwrap();
    match writer.as_deref() {
        None => {
            *writer = Some(id);
            Ok(())
        }
        Some(current) if current == id => Ok(()),
        Some(_) => Err(err(
            StatusCode::LOCKED,
            "another client holds the tuning lock",
        )),
    }
}

pub async fn lock_release(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let id = client_id(&headers);
    let mut writer = state.writer.lock().unwrap();
    if writer.as_deref() == Some(id.as_str()) {
        *writer = None;
    }
    Json(serde_json::json!({ "writer": *writer })).into_response()
}

fn broadcast_tune(state: &SharedState) {
    let msg = comms::tune_message(&state.tune.lock().unwrap());
    let _ = state.events.send(msg);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub id: String,
    pub title: String,
}

pub async fn tune_summary(State(state): State<SharedState>) -> Response {
    let tune = state.tune.lock().unwrap();
    let tables: Vec<TableInfo> = state
        .def
        .tables
        .iter()
        .filter(|(id, _)| tune.table(id).is_some())
        .map(|(id, t)| TableInfo {
            id: id.clone(),
            title: t.title.clone(),
        })
        .collect();
    Json(serde_json::json!({
        "loaded": tune.loaded(),
        "dirty": tune.any_dirty(),
        "burnPending": tune.burn_pending(),
        "writer": *state.writer.lock().unwrap(),
        "tables": tables,
    }))
    .into_response()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TableJson {
    id: String,
    title: String,
    x: Vec<f64>,
    y: Vec<f64>,
    z: Vec<Vec<f64>>,
    z_lo: f64,
    z_hi: f64,
    z_digits: u8,
    x_label: Option<String>,
    y_label: Option<String>,
    x_channel: Option<String>,
    y_channel: Option<String>,
}

pub async fn tune_table(State(state): State<SharedState>, Path(id): Path<String>) -> Response {
    let tune = state.tune.lock().unwrap();
    if !tune.loaded() {
        return err(StatusCode::CONFLICT, "tune not loaded").into_response();
    }
    let Some(def) = state.def.tables.get(&id) else {
        return err(StatusCode::NOT_FOUND, format!("unknown table `{id}`")).into_response();
    };
    let Some(data) = tune.table(&id) else {
        return err(
            StatusCode::CONFLICT,
            format!("table `{id}` does not decode"),
        )
        .into_response();
    };
    Json(TableJson {
        id: id.clone(),
        title: def.title.clone(),
        x: data.x,
        y: data.y,
        z: data.z,
        z_lo: data.z_lo,
        z_hi: data.z_hi,
        z_digits: data.z_digits,
        x_label: def.xy_labels.first().cloned(),
        y_label: def.xy_labels.get(1).cloned(),
        x_channel: def.x_bins.1.clone(),
        y_channel: def.y_bins.1.clone(),
    })
    .into_response()
}

#[derive(Deserialize)]
pub struct CellEdit {
    pub row: usize,
    pub col: usize,
    pub value: f64,
}

#[derive(Deserialize)]
pub struct CellsReq {
    pub cells: Vec<CellEdit>,
}

pub async fn tune_table_cells(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CellsReq>,
) -> Response {
    if let Err(e) = acquire_writer(&state, &headers) {
        return e.into_response();
    }
    {
        let mut tune = state.tune.lock().unwrap();
        if !tune.loaded() {
            return err(StatusCode::CONFLICT, "tune not loaded").into_response();
        }
        for cell in &req.cells {
            if let Err(e) = tune.set_table_cell(&id, cell.row, cell.col, cell.value) {
                return err(StatusCode::BAD_REQUEST, e.to_string()).into_response();
            }
        }
    }
    broadcast_tune(&state);
    Json(serde_json::json!({ "ok": true })).into_response()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConstantJson {
    name: String,
    value: serde_json::Value,
    units: Option<String>,
    digits: u8,
    lo: Option<f64>,
    hi: Option<f64>,
    /// Combo labels for bits constants (INVALID entries are hidden).
    labels: Vec<String>,
    requires_power_cycle: bool,
}

fn constant_json(state: &AppState, tune: &Tune, name: &str) -> Option<ConstantJson> {
    // Page constants (ECU bytes) or PcVariables (app-side settings like
    // gauge limits — no power cycle, no burn).
    let (c, value, requires_power_cycle) = if let Some(c) = state.def.constants.get(name) {
        let value = match tune.constant_value(name)? {
            Value::Num(n) => serde_json::json!(n),
            Value::Str(s) => serde_json::json!(s),
        };
        (c, value, tune.requires_power_cycle(name))
    } else {
        let c = state.def.pc_variables.get(name)?;
        (c, serde_json::json!(tune.pc_value(name)?), false)
    };
    Some(ConstantJson {
        name: name.to_string(),
        value,
        units: c.units.as_ref().and_then(|u| u.eval(tune).ok()),
        digits: c
            .digits
            .as_ref()
            .and_then(|d| d.eval(tune).ok())
            .unwrap_or(0.0) as u8,
        lo: c.lo.as_ref().and_then(|v| v.eval(tune).ok()),
        hi: c.hi.as_ref().and_then(|v| v.eval(tune).ok()),
        labels: c.labels.clone(),
        requires_power_cycle,
    })
}

#[derive(Deserialize)]
pub struct NamesQuery {
    pub names: String,
}

pub async fn tune_constants(
    State(state): State<SharedState>,
    Query(query): Query<NamesQuery>,
) -> Response {
    let tune = state.tune.lock().unwrap();
    if !tune.loaded() {
        return err(StatusCode::CONFLICT, "tune not loaded").into_response();
    }
    let list: Vec<ConstantJson> = query
        .names
        .split(',')
        .filter_map(|name| constant_json(&state, &tune, name.trim()))
        .collect();
    Json(list).into_response()
}

#[derive(Deserialize)]
pub struct SetConstantReq {
    pub value: f64,
}

pub async fn tune_set_constant(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SetConstantReq>,
) -> Response {
    if let Err(e) = acquire_writer(&state, &headers) {
        return e.into_response();
    }
    // PcVariables (gauge limits, ...) are app-side: no serial write, no
    // dirty tracking — but gauge bounds may depend on them.
    let is_pc =
        !state.def.constants.contains_key(&name) && state.def.pc_variables.contains_key(&name);
    let response = {
        let mut tune = state.tune.lock().unwrap();
        if !tune.loaded() {
            return err(StatusCode::CONFLICT, "tune not loaded").into_response();
        }
        let result = if is_pc {
            tune.set_pc_value(&name, req.value)
        } else {
            tune.set_constant(&name, req.value)
        };
        if let Err(e) = result {
            return err(StatusCode::BAD_REQUEST, e.to_string()).into_response();
        }
        if is_pc {
            refresh_definition(&state, &tune);
        }
        constant_json(&state, &tune, &name)
    };
    if !is_pc {
        broadcast_tune(&state);
    }
    match response {
        Some(c) => Json(c).into_response(),
        None => err(StatusCode::INTERNAL_SERVER_ERROR, "constant vanished").into_response(),
    }
}

// ----- INI settings dialogs: menu tree + generated forms --------------------

/// Evaluate an optional enable/visible condition against the live tune.
/// Unresolvable conditions default to `true` — never hide a setting we
/// can't reason about.
fn truthy(cond: Option<&ts_ini::Expr>, tune: &Tune) -> bool {
    match cond {
        None => true,
        Some(e) => match e.eval(tune) {
            Ok(Value::Num(n)) => n != 0.0,
            Ok(Value::Str(s)) => !s.is_empty(),
            Err(_) => true,
        },
    }
}

/// A [Menu] entry, classified by what its target resolves to; `None` for
/// TunerStudio built-ins (`std_*`) and 3D map views we don't serve.
fn menu_entry_json(
    state: &AppState,
    tune: &Tune,
    e: &ts_ini::MenuEntry,
) -> Option<serde_json::Value> {
    let kind = if state.def.dialogs.contains_key(&e.target) {
        "dialog"
    } else if state.def.tables.contains_key(&e.target) {
        "table"
    } else if state.def.curves.contains_key(&e.target) {
        "curve"
    } else {
        return None;
    };
    Some(serde_json::json!({
        "type": kind,
        "name": e.target,
        "label": e.label,
        "enabled": truthy(e.enable.as_ref(), tune),
    }))
}

pub async fn tune_menus(State(state): State<SharedState>) -> Response {
    let tune = state.tune.lock().unwrap();
    if !tune.loaded() {
        return err(StatusCode::CONFLICT, "tune not loaded").into_response();
    }
    let menus: Vec<serde_json::Value> = state
        .def
        .menus
        .iter()
        .filter_map(|menu| {
            let mut items: Vec<serde_json::Value> = Vec::new();
            for item in &menu.items {
                match item {
                    MenuItem::Entry(e) => items.extend(menu_entry_json(&state, &tune, e)),
                    MenuItem::Group { label, children } => {
                        let kids: Vec<serde_json::Value> = children
                            .iter()
                            .filter_map(|e| menu_entry_json(&state, &tune, e))
                            .collect();
                        if !kids.is_empty() {
                            items.push(serde_json::json!({
                                "type": "group", "label": label, "items": kids,
                            }));
                        }
                    }
                    // Only between surviving entries, never doubled.
                    MenuItem::Separator => {
                        if matches!(items.last(), Some(v) if v["type"] != "separator") {
                            items.push(serde_json::json!({ "type": "separator" }));
                        }
                    }
                }
            }
            while matches!(items.last(), Some(v) if v["type"] == "separator") {
                items.pop();
            }
            (!items.is_empty()).then(|| serde_json::json!({ "title": menu.title, "items": items }))
        })
        .collect();
    Json(menus).into_response()
}

/// Flatten a dialog's fields and nested panels into renderable form rows.
/// `visited` holds the ancestor chain to break panel cycles.
fn dialog_items(
    state: &AppState,
    tune: &Tune,
    dialog: &ts_ini::DialogDef,
    visited: &mut Vec<String>,
) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for item in &dialog.items {
        match item {
            DialogItem::Field {
                label,
                constant,
                enable,
                visible,
            } => {
                if !truthy(visible.as_ref(), tune) {
                    continue;
                }
                match constant {
                    None => {
                        // `#` marks a TS bold header, `!"..."` red note text;
                        // blank labels are spacers.
                        let text = label
                            .trim_start_matches(['#', '!'])
                            .trim()
                            .trim_matches('"')
                            .trim();
                        if !text.is_empty() {
                            out.push(serde_json::json!({ "type": "header", "label": text }));
                        }
                    }
                    Some(name) => match constant_json(state, tune, name) {
                        Some(cj) => out.push(serde_json::json!({
                            "type": "constant",
                            "label": label,
                            "enabled": truthy(enable.as_ref(), tune),
                            "constant": cj,
                        })),
                        // PcVariables and channels aren't editable tune bytes.
                        None => out.push(serde_json::json!({
                            "type": "unsupported", "label": label, "name": name,
                        })),
                    },
                }
            }
            DialogItem::Panel { target, enable } => {
                if visited.iter().any(|v| v == target) {
                    continue;
                }
                if let Some(sub) = state.def.dialogs.get(target) {
                    visited.push(target.clone());
                    let items = dialog_items(state, tune, sub, visited);
                    visited.pop();
                    if !items.is_empty() {
                        out.push(serde_json::json!({
                            "type": "panel",
                            "name": target,
                            "title": sub.title,
                            "enabled": truthy(enable.as_ref(), tune),
                            "items": items,
                        }));
                    }
                } else if let Some(curve) = state.def.curves.get(target) {
                    out.push(serde_json::json!({
                        "type": "curve", "name": target, "title": curve.title,
                    }));
                } else if let Some(table) = state.def.tables.get(target) {
                    out.push(serde_json::json!({
                        "type": "table", "name": target, "title": table.title,
                    }));
                }
                // Live graphs and other visual panel targets: skipped.
            }
        }
    }
    out
}

/// The label a [Menu] entry gives this target — the fallback title for
/// dialogs defined with an empty one (`dialog = engine_constants, ""`).
fn menu_label(def: &ts_ini::IniDef, target: &str) -> Option<String> {
    def.menus
        .iter()
        .flat_map(|m| m.items.iter())
        .find_map(|item| match item {
            MenuItem::Entry(e) if e.target == target => Some(e.label.clone()),
            MenuItem::Group { children, .. } => children
                .iter()
                .find(|e| e.target == target)
                .map(|e| e.label.clone()),
            _ => None,
        })
}

pub async fn tune_dialog(State(state): State<SharedState>, Path(name): Path<String>) -> Response {
    let tune = state.tune.lock().unwrap();
    if !tune.loaded() {
        return err(StatusCode::CONFLICT, "tune not loaded").into_response();
    }
    let Some(dialog) = state.def.dialogs.get(&name) else {
        return err(StatusCode::NOT_FOUND, format!("no dialog `{name}`")).into_response();
    };
    let title = if dialog.title.is_empty() {
        menu_label(&state.def, &name).unwrap_or_else(|| dialog.name.clone())
    } else {
        dialog.title.clone()
    };
    let mut visited = vec![name.clone()];
    let items = dialog_items(&state, &tune, dialog, &mut visited);
    Json(serde_json::json!({
        "name": dialog.name,
        "title": title,
        "help": dialog.topic_help,
        "items": items,
    }))
    .into_response()
}

// ----- offline mode: edit an opened .msq with no ECU ------------------------

/// Open the uploaded .msq as an offline tune: full editing (tables,
/// settings, diff, save) with no serial connection. `dirty` then means
/// "changed since the file was opened".
pub async fn offline_open(State(state): State<SharedState>) -> Response {
    if state.status.lock().unwrap().connected {
        return err(
            StatusCode::CONFLICT,
            "connected to an ECU — disconnect first",
        )
        .into_response();
    }
    let report = {
        let msq = state.msq.lock().unwrap();
        let Some((_, file)) = msq.as_ref() else {
            return err(StatusCode::CONFLICT, "no .msq uploaded").into_response();
        };
        let mut tune = state.tune.lock().unwrap();
        let mut fresh = Tune::new(state.def.clone());
        fresh.adopt_pc_values(&tune);
        let report = tune_model::msq::apply(file, &mut fresh, None);
        fresh.sync_shadows();
        fresh.set_loaded(true);
        *tune = fresh;
        refresh_definition(&state, &tune);
        report
    };
    *state.writer.lock().unwrap() = None;
    let status = {
        let mut status = state.status.lock().unwrap();
        *status = Status {
            offline: true,
            tune_loaded: true,
            ..Status::default()
        };
        status.clone()
    };
    let _ = state.events.send(comms::status_message(&status));
    broadcast_tune(&state);
    Json(serde_json::json!({
        "status": status,
        "applied": report.applied,
        "skipped": report.skipped,
    }))
    .into_response()
}

/// End the offline session, discarding the working tune.
pub async fn offline_close(State(state): State<SharedState>) -> Response {
    if !state.status.lock().unwrap().offline {
        return err(StatusCode::CONFLICT, "not in offline mode").into_response();
    }
    {
        let mut tune = state.tune.lock().unwrap();
        let mut fresh = Tune::new(state.def.clone());
        fresh.adopt_pc_values(&tune);
        *tune = fresh;
    }
    *state.writer.lock().unwrap() = None;
    let status = {
        let mut status = state.status.lock().unwrap();
        *status = Status::default();
        status.clone()
    };
    let _ = state.events.send(comms::status_message(&status));
    broadcast_tune(&state);
    Json(status).into_response()
}

// ----- .msq reference file: upload, diff, selective apply, save -------------

#[derive(Deserialize)]
pub struct MsqUploadReq {
    pub filename: String,
    /// Full file text (client decodes ISO-8859-1 before sending).
    pub content: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MsqMeta {
    pub filename: String,
    pub signature: Option<String>,
    pub signature_match: bool,
    pub write_date: Option<String>,
    pub author: Option<String>,
    pub settings: Vec<String>,
    pub constants: usize,
}

fn msq_meta(state: &AppState, filename: &str, file: &tune_model::MsqFile) -> MsqMeta {
    MsqMeta {
        filename: filename.to_string(),
        signature: file.signature.clone(),
        signature_match: file.signature.as_deref() == Some(state.def.signature.as_str()),
        write_date: file.write_date.clone(),
        author: file.author.clone(),
        settings: file.settings.clone(),
        constants: file.constants.len(),
    }
}

pub async fn msq_upload(
    State(state): State<SharedState>,
    Json(req): Json<MsqUploadReq>,
) -> Response {
    let file = match tune_model::msq::parse(&req.content) {
        Ok(file) => file,
        Err(e) => return err(StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    if file.constants.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "no constants found — not a .msq tune?",
        )
        .into_response();
    }
    let meta = msq_meta(&state, &req.filename, &file);
    *state.msq.lock().unwrap() = Some((req.filename, file));
    Json(meta).into_response()
}

/// "Where" label for a constant: the table/curve using it, else its page.
fn where_label(state: &AppState, name: &str, page: Option<u8>) -> String {
    for table in state.def.tables.values() {
        let role = if table.z_bins == name {
            ""
        } else if table.x_bins.0 == name {
            " (X axis)"
        } else if table.y_bins.0 == name {
            " (Y axis)"
        } else {
            continue;
        };
        return format!("{}{}", table.title, role);
    }
    for curve in state.def.curves.values() {
        if curve.x_bins.0 == name || curve.y_bins.iter().any(|b| b == name) {
            return curve.title.clone();
        }
    }
    page.map(|p| format!("page {p}")).unwrap_or_default()
}

/// Per-cell detail cap: enough to show "where", without shipping whole maps.
const MAX_DIFF_CELLS: usize = 64;

fn diff_json(state: &AppState, tune: &Tune) -> Result<serde_json::Value, ApiError> {
    let msq = state.msq.lock().unwrap();
    let Some((filename, file)) = msq.as_ref() else {
        return Err(err(StatusCode::CONFLICT, "no .msq uploaded"));
    };
    let diff = tune_model::msq::diff(file, tune);

    let mut entries = Vec::new();
    for entry in &diff.entries {
        let where_ = where_label(state, &entry.name, entry.page);
        let base = serde_json::json!({
            "name": entry.name,
            "page": entry.page,
            "where": where_,
        });
        let mut obj = base.as_object().unwrap().clone();
        match &entry.kind {
            tune_model::DiffKind::Scalar { ecu, file } => {
                obj.insert("kind".into(), "scalar".into());
                obj.insert("ecu".into(), serde_json::json!(ecu));
                obj.insert("file".into(), serde_json::json!(file));
            }
            tune_model::DiffKind::Bits { ecu, file } => {
                obj.insert("kind".into(), "bits".into());
                obj.insert("ecu".into(), serde_json::json!(ecu));
                obj.insert("file".into(), serde_json::json!(file));
            }
            tune_model::DiffKind::Array { changed, len } => {
                obj.insert("kind".into(), "array".into());
                obj.insert("changedCount".into(), serde_json::json!(changed.len()));
                obj.insert("len".into(), serde_json::json!(len));
                // Per-element detail with values; 2D shapes get row/col.
                let ecu_values = tune.array_values(&entry.name).unwrap_or_default();
                let file_values = match file.constants.get(&entry.name) {
                    Some(tune_model::MsqValue::Array(v)) => v.clone(),
                    _ => Vec::new(),
                };
                let nx = state
                    .def
                    .constants
                    .get(&entry.name)
                    .and_then(|c| match c.shape {
                        Some(ts_ini::Shape::Array2D { x, .. }) => Some(x as usize),
                        _ => None,
                    });
                let cells: Vec<serde_json::Value> = changed
                    .iter()
                    .take(MAX_DIFF_CELLS)
                    .map(|&i| {
                        let mut cell = serde_json::Map::new();
                        cell.insert("index".into(), serde_json::json!(i));
                        if let Some(nx) = nx {
                            cell.insert("row".into(), serde_json::json!(i / nx));
                            cell.insert("col".into(), serde_json::json!(i % nx));
                        }
                        cell.insert("ecu".into(), serde_json::json!(ecu_values.get(i)));
                        cell.insert("file".into(), serde_json::json!(file_values.get(i)));
                        serde_json::Value::Object(cell)
                    })
                    .collect();
                obj.insert("cells".into(), serde_json::Value::Array(cells));
            }
        }
        entries.push(serde_json::Value::Object(obj));
    }

    Ok(serde_json::json!({
        "meta": msq_meta(state, filename, file),
        "entries": entries,
        "onlyInFile": diff.only_in_file,
        "unresolved": diff.unresolved,
    }))
}

pub async fn msq_diff(State(state): State<SharedState>) -> Response {
    let tune = state.tune.lock().unwrap();
    if !tune.loaded() {
        return err(StatusCode::CONFLICT, "tune not loaded").into_response();
    }
    match diff_json(&state, &tune) {
        Ok(json) => Json(json).into_response(),
        Err(e) => e.into_response(),
    }
}

#[derive(Deserialize)]
pub struct MsqApplyReq {
    /// Restrict to these constants (selective send); omit for all.
    pub names: Option<Vec<String>>,
}

pub async fn msq_apply(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(req): Json<MsqApplyReq>,
) -> Response {
    if let Err(e) = acquire_writer(&state, &headers) {
        return e.into_response();
    }
    let report = {
        let msq = state.msq.lock().unwrap();
        let Some((_, file)) = msq.as_ref() else {
            return err(StatusCode::CONFLICT, "no .msq uploaded").into_response();
        };
        let mut tune = state.tune.lock().unwrap();
        if !tune.loaded() {
            return err(StatusCode::CONFLICT, "tune not loaded").into_response();
        }
        let names: Option<std::collections::HashSet<String>> =
            req.names.map(|n| n.into_iter().collect());
        let report = tune_model::msq::apply(file, &mut tune, names.as_ref());
        // The file may carry PcVariables (gauge limits) — re-resolve gauges.
        refresh_definition(&state, &tune);
        report
    };
    broadcast_tune(&state);
    Json(serde_json::json!({
        "applied": report.applied,
        "skipped": report.skipped,
    }))
    .into_response()
}

pub async fn msq_save(State(state): State<SharedState>) -> Response {
    let tune = state.tune.lock().unwrap();
    if !tune.loaded() {
        return err(StatusCode::CONFLICT, "tune not loaded").into_response();
    }
    let now = chrono::Local::now();
    let content = tune_model::msq::save(
        &tune,
        &state.symbols,
        &format!("rustytune {}", env!("CARGO_PKG_VERSION")),
        &now.format("%a %b %d %H:%M:%S %Y").to_string(),
    );
    let filename = format!("rustytune_{}.msq", now.format("%Y%m%d_%H%M%S"));
    (
        [
            (header::CONTENT_TYPE, "application/xml".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        content,
    )
        .into_response()
}

pub async fn tune_burn(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    if let Err(e) = acquire_writer(&state, &headers) {
        return e.into_response();
    }
    let result =
        tokio::task::spawn_blocking(move || comms_roundtrip(&state, |reply| Cmd::Burn { reply }))
            .await;
    match result {
        Ok(Ok(Ok(pages))) => Json(serde_json::json!({ "burnedPages": pages })).into_response(),
        Ok(Ok(Err(msg))) => err(StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
        Ok(Err(resp)) => resp.into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_name_filter() {
        assert!(is_serial_name("cu.usbserial-1420"));
        assert!(is_serial_name("cu.Bluetooth-Incoming-Port"));
        assert!(is_serial_name("ttyUSB0"));
        assert!(is_serial_name("ttyACM3"));
        assert!(!is_serial_name("tty.usbserial-1420")); // dial-in: prefer cu.*
        assert!(!is_serial_name("disk3"));
        assert!(!is_serial_name("null"));

        assert!(looks_usb("cu.usbserial-1420"));
        assert!(looks_usb("cu.usbmodem101"));
        assert!(looks_usb("ttyUSB0"));
        assert!(!looks_usb("cu.Bluetooth-Incoming-Port"));
    }
}
