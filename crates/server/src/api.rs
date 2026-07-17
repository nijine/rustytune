//! REST + WebSocket handlers. All tune/telemetry logic stays server-side;
//! the frontend is a thin renderer of these responses.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    Json,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use ecu_proto::{Config, Mode, SerialTransport, Session};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::comms::{self, Cmd, CommsCtx, CommsHandle, Status};
use crate::definition::{Defaults, DefinitionUi};

pub struct AppState {
    pub def: Arc<ts_ini::IniDef>,
    pub defaults: Arc<Defaults>,
    pub definition: DefinitionUi,
    pub status: Arc<Mutex<Status>>,
    pub events: broadcast::Sender<String>,
    pub comms: Mutex<Option<CommsHandle>>,
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
    Json(out)
}

pub async fn status(State(state): State<SharedState>) -> Json<Status> {
    Json(state.status.lock().unwrap().clone())
}

pub async fn definition(State(state): State<SharedState>) -> Json<DefinitionUi> {
    Json(state.definition.clone())
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

    let ctx = CommsCtx {
        def: state.def.clone(),
        defaults: state.defaults.clone(),
        status: state.status.clone(),
        events: state.events.clone(),
        poll_interval: Duration::from_millis(req.poll_ms.clamp(20, 1000)),
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
