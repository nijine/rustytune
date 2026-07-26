//! The comms thread: owns the serial `Session`, polls realtime frames,
//! evaluates indicators, writes the datalog, and broadcasts JSON messages.
//!
//! Everything blocking lives here on a plain OS thread; the async side talks
//! to it through a command channel and a tokio broadcast of pre-serialized
//! JSON strings.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use datalog::MslWriter;
use ecu_proto::{PageCommands, PagesConfig, ProtoError, SerialTransport, Session};
use serde::Serialize;
use tokio::sync::broadcast;
use ts_ini::{IniDef, SymbolSource, Telemetry, Value};
use tune_model::Tune;

use crate::config::EngineShutdownConfig;
use crate::definition::Defaults;

pub enum Cmd {
    StartLog {
        path: PathBuf,
        reply: mpsc::Sender<Result<PathBuf, String>>,
    },
    StopLog {
        reply: mpsc::Sender<Option<LogSummary>>,
    },
    /// Flush pending edits, then burn every page whose ECU RAM differs
    /// from EEPROM. Replies with the burned page indices.
    Burn {
        reply: mpsc::Sender<Result<Vec<usize>, String>>,
    },
    Shutdown,
}

/// Gaps up to this many bytes merge into one `M` write.
const SPAN_MERGE_GAP: usize = 4;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSummary {
    pub path: PathBuf,
    pub rows: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LogStatus {
    pub path: String,
    pub rows: u64,
}

/// Connection status snapshot, shared with the async side and serialized
/// into `{"type":"status",...}` broadcasts.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub connected: bool,
    pub port: Option<String>,
    pub mode: Option<String>,
    pub baud: Option<u32>,
    pub frames: u64,
    pub crc_errors: u64,
    pub timeouts: u64,
    /// What the ECU answered to the INI's queryCommand (primary mode only).
    pub ecu_signature: Option<String>,
    /// All tune pages downloaded and CRC-verified.
    pub tune_loaded: bool,
    /// Editing an opened .msq with no ECU — tune loaded, no serial.
    pub offline: bool,
    pub last_error: Option<String>,
    pub log: Option<LogStatus>,
}

pub struct CommsHandle {
    pub cmd_tx: mpsc::Sender<Cmd>,
    pub join: JoinHandle<()>,
}

pub struct CommsCtx {
    pub def: Arc<IniDef>,
    pub defaults: Arc<Defaults>,
    pub status: Arc<Mutex<Status>>,
    pub events: broadcast::Sender<String>,
    pub poll_interval: Duration,
    pub tune: Arc<Mutex<Tune>>,
    /// Page command set; `None` on the secondary serial (telemetry-only).
    pub pages: Option<PageCommands>,
    pub auto_log: bool,
    pub log_dir: PathBuf,
    pub retention_bytes: u64,
    pub engine_shutdown: EngineShutdownConfig,
    pub shutdown_request_path: Option<PathBuf>,
}

/// Build the page command set from the INI header (primary mode).
pub fn page_commands(def: &IniDef) -> Result<PageCommands, ProtoError> {
    let sizes = &def.header.page_sizes;
    PageCommands::new(&PagesConfig {
        identifiers: &def.header.page_identifiers,
        page_read: &def.header.page_read_command,
        chunk_write: &def.header.page_chunk_write,
        crc_check: &def.header.crc32_check_command,
        burn: &def.header.burn_command,
        sizes,
        blocking_factor: def.header.blocking_factor.unwrap_or(251) as u16,
        can_id: 0,
    })
}

/// `{"type":"tune",...}` — dirty/burn state for all clients.
pub fn tune_message(tune: &Tune) -> String {
    serde_json::json!({
        "type": "tune",
        "loaded": tune.loaded(),
        "dirty": tune.any_dirty(),
        "burnPending": tune.burn_pending(),
    })
    .to_string()
}

fn broadcast_tune(ctx: &CommsCtx) {
    let msg = tune_message(&ctx.tune.lock().unwrap());
    let _ = ctx.events.send(msg);
}

pub fn status_message(status: &Status) -> String {
    #[derive(Serialize)]
    struct Msg<'a> {
        r#type: &'static str,
        #[serde(flatten)]
        status: &'a Status,
    }
    serde_json::to_string(&Msg {
        r#type: "status",
        status,
    })
    .expect("status serializes")
}

fn broadcast_status(ctx: &CommsCtx) {
    let msg = status_message(&ctx.status.lock().unwrap());
    let _ = ctx.events.send(msg);
}

/// Identifier fallback for derived channels and indicator conditions:
/// `timeNow` plus `[DefaultValues]` (for tune constants like `stoich`).
struct Extra<'a> {
    defaults: &'a Defaults,
    t: f64,
}

impl SymbolSource for Extra<'_> {
    fn value(&self, name: &str) -> Option<Value> {
        if name == "timeNow" {
            return Some(Value::Num(self.t));
        }
        self.defaults.value(name)
    }
}

fn round6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

/// Decode every output channel and indicator into one frame message; also
/// returns the numeric channel values for datalog columns.
fn build_frame(
    ctx: &CommsCtx,
    block: &[u8],
    t: f64,
    log_rows: Option<u64>,
) -> (String, std::collections::HashMap<String, f64>) {
    let extra = Extra {
        defaults: &ctx.defaults,
        t,
    };
    let telemetry = Telemetry::with_extra(&ctx.def, block, &extra);

    let mut channels = serde_json::Map::with_capacity(ctx.def.output_channels.len());
    let mut numeric = std::collections::HashMap::with_capacity(ctx.def.output_channels.len());
    for name in ctx.def.output_channels.keys() {
        match telemetry.channel(name) {
            Some(Value::Num(n)) if n.is_finite() => {
                numeric.insert(name.clone(), n);
                channels.insert(name.clone(), serde_json::json!(round6(n)));
            }
            Some(Value::Str(s)) => {
                channels.insert(name.clone(), serde_json::json!(s));
            }
            _ => {} // undecodable this frame (e.g. depends on tune data)
        }
    }

    let indicators: Vec<bool> = ctx
        .def
        .front_page
        .indicators
        .iter()
        .map(|ind| match ind.condition.eval(&telemetry) {
            Ok(Value::Num(n)) => n != 0.0,
            _ => false,
        })
        .collect();

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Frame<'a> {
        r#type: &'static str,
        t: f64,
        channels: &'a serde_json::Map<String, serde_json::Value>,
        indicators: &'a [bool],
        #[serde(skip_serializing_if = "Option::is_none")]
        log_rows: Option<u64>,
    }
    let json = serde_json::to_string(&Frame {
        r#type: "frame",
        t: round6(t),
        channels: &channels,
        indicators: &indicators,
        log_rows,
    })
    .expect("frame serializes");
    (json, numeric)
}

/// After this many consecutive unanswered polls the status shows an error
/// (polling continues — the ECU may come back after a power cycle).
const SILENT_POLLS_BEFORE_ERROR: u32 = 3;

struct EngineShutdownMonitor {
    config: EngineShutdownConfig,
    armed: bool,
    stopped_since: Option<Instant>,
    requested: bool,
}

impl EngineShutdownMonitor {
    fn new(config: EngineShutdownConfig) -> Self {
        Self {
            config,
            armed: false,
            stopped_since: None,
            requested: false,
        }
    }

    fn observe(&mut self, rpm: Option<f64>, now: Instant) -> bool {
        if !self.config.enabled || self.requested {
            return false;
        }
        let Some(rpm) = rpm.filter(|value| value.is_finite()) else {
            self.stopped_since = None;
            return false;
        };
        if rpm >= self.config.arm_rpm {
            self.armed = true;
            self.stopped_since = None;
            return false;
        }
        if !self.armed || rpm > self.config.stop_rpm {
            self.stopped_since = None;
            return false;
        }
        let since = self.stopped_since.get_or_insert(now);
        if now.duration_since(*since) >= Duration::from_secs(self.config.delay_seconds) {
            self.requested = true;
            return true;
        }
        false
    }
}

pub fn spawn(
    session: Session<SerialTransport>,
    ctx: CommsCtx,
    delay_after_open: Option<Duration>,
) -> CommsHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let join = std::thread::Builder::new()
        .name("comms".into())
        .spawn(move || run(session, ctx, cmd_rx, delay_after_open))
        .expect("spawn comms thread");
    CommsHandle { cmd_tx, join }
}

fn run(
    mut session: Session<SerialTransport>,
    ctx: CommsCtx,
    cmd_rx: mpsc::Receiver<Cmd>,
    delay_after_open: Option<Duration>,
) {
    // Boards that reset on port open (DTR) need a beat before the first
    // command; the INI's delayAfterPortOpen.
    if let Some(delay) = delay_after_open {
        std::thread::sleep(delay);
    }

    // Verify we're talking to the firmware this INI describes. A mismatch
    // is surfaced but not fatal — telemetry offsets may still line up, and
    // the user may be probing which INI they need.
    if session.config().mode == ecu_proto::Mode::Primary {
        match session.query_string(&ctx.def.query_command) {
            Ok(signature) => {
                let mut status = ctx.status.lock().unwrap();
                if signature != ctx.def.signature {
                    tracing::warn!("ECU signature `{signature}` != INI `{}`", ctx.def.signature);
                    status.last_error = Some(format!(
                        "signature mismatch: ECU says `{signature}`, INI is `{}`",
                        ctx.def.signature
                    ));
                }
                status.ecu_signature = Some(signature);
                drop(status);
                broadcast_status(&ctx);
            }
            Err(e) => tracing::warn!("signature query failed: {e}"),
        }
    }

    // Download every tune page and CRC-verify it (primary only) — the
    // browser's table editors work on this snapshot.
    if let Some(pages) = &ctx.pages {
        match download_tune(&mut session, pages, &ctx) {
            Ok(()) => {
                ctx.tune.lock().unwrap().set_loaded(true);
                ctx.status.lock().unwrap().tune_loaded = true;
                broadcast_status(&ctx);
                broadcast_tune(&ctx);
            }
            Err(e) => {
                tracing::error!("tune download failed: {e}");
                ctx.status.lock().unwrap().last_error = Some(format!("tune download failed: {e}"));
                broadcast_status(&ctx);
            }
        }
    }

    let start = Instant::now();
    let mut log: Option<MslWriter> = None;
    let mut auto_log_enabled = ctx.auto_log;
    let mut shutdown_monitor = EngineShutdownMonitor::new(ctx.engine_shutdown.clone());
    let mut consecutive_timeouts = 0u32;
    let mut next_poll = Instant::now();

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Cmd::Shutdown => {
                    finish_log(&ctx, &mut log);
                    ctx.tune.lock().unwrap().set_loaded(false);
                    let mut status = ctx.status.lock().unwrap();
                    status.connected = false;
                    status.tune_loaded = false;
                    status.log = None;
                    drop(status);
                    broadcast_status(&ctx);
                    broadcast_tune(&ctx);
                    return;
                }
                Cmd::StartLog { path, reply } => {
                    let result = start_log(&ctx, &path);
                    match result {
                        Ok(writer) => {
                            ctx.status.lock().unwrap().log = Some(LogStatus {
                                path: path.display().to_string(),
                                rows: 0,
                            });
                            log = Some(writer);
                            let _ = reply.send(Ok(path));
                        }
                        Err(e) => {
                            let _ = reply.send(Err(e.to_string()));
                        }
                    }
                    broadcast_status(&ctx);
                }
                Cmd::StopLog { reply } => {
                    // A manual stop pauses automatic logging until the next
                    // ECU connection/session.
                    auto_log_enabled = false;
                    let summary = finish_log(&ctx, &mut log);
                    let _ = reply.send(summary);
                    broadcast_status(&ctx);
                }
                Cmd::Burn { reply } => {
                    let _ = reply.send(do_burn(&mut session, &ctx));
                    broadcast_tune(&ctx);
                }
            }
        }

        // Push pending tune edits to the ECU promptly (between polls).
        if let Some(pages) = &ctx.pages {
            flush_dirty(&mut session, pages, &ctx);
        }

        let now = Instant::now();
        if now < next_poll {
            std::thread::sleep((next_poll - now).min(Duration::from_millis(20)));
            continue;
        }
        // Keep cadence, but never schedule into the past after a stall.
        next_poll = (next_poll + ctx.poll_interval).max(now);

        match session.read_realtime() {
            Ok(block) => {
                if auto_log_enabled && log.is_none() {
                    let path = ctx.log_dir.join(format!(
                        "rustytune_{}.msl",
                        chrono::Local::now().format("%Y%m%d_%H%M%S")
                    ));
                    match start_log(&ctx, &path) {
                        Ok(writer) => {
                            ctx.status.lock().unwrap().log = Some(LogStatus {
                                path: path.display().to_string(),
                                rows: 0,
                            });
                            log = Some(writer);
                        }
                        Err(e) => {
                            ctx.status.lock().unwrap().last_error =
                                Some(format!("automatic logging: {e}"))
                        }
                    }
                }
                let had_error = consecutive_timeouts >= SILENT_POLLS_BEFORE_ERROR;
                consecutive_timeouts = 0;
                let t = start.elapsed().as_secs_f64();
                let (json, numeric) = build_frame(&ctx, &block, t, log.as_ref().map(|w| w.rows()));

                if let Some(writer) = &mut log {
                    let row: Vec<Option<f64>> = writer
                        .columns()
                        .iter()
                        .map(|c| numeric.get(&c.channel).copied())
                        .collect();
                    if let Err(e) = writer.write_row(&row) {
                        tracing::error!("datalog write failed: {e}");
                        ctx.status.lock().unwrap().last_error =
                            Some(format!("datalog write failed: {e}"));
                        finish_log(&ctx, &mut log);
                        broadcast_status(&ctx);
                    }
                }

                {
                    let mut status = ctx.status.lock().unwrap();
                    status.frames += 1;
                    if let (Some(ls), Some(writer)) = (&mut status.log, &log) {
                        ls.rows = writer.rows();
                    }
                    if had_error {
                        status.last_error = None;
                    }
                }
                if had_error {
                    broadcast_status(&ctx); // recovered
                }
                let _ = ctx.events.send(json);

                if shutdown_monitor.observe(numeric.get("rpm").copied(), Instant::now()) {
                    tracing::info!(
                        "engine stopped; closing datalog and requesting appliance shutdown"
                    );
                    finish_log(&ctx, &mut log);
                    broadcast_status(&ctx);
                    let request = ctx
                        .shutdown_request_path
                        .as_ref()
                        .ok_or_else(|| "shutdown request path is not configured".to_owned())
                        .and_then(|path| {
                            std::fs::write(path, b"shutdown\n")
                                .map_err(|e| format!("write {}: {e}", path.display()))
                        });
                    if let Err(e) = request {
                        tracing::error!("automatic shutdown request failed: {e}");
                        ctx.status.lock().unwrap().last_error =
                            Some(format!("automatic shutdown request failed: {e}"));
                        broadcast_status(&ctx);
                    }
                    return;
                }
            }
            Err(ProtoError::CrcMismatch) => {
                ctx.status.lock().unwrap().crc_errors += 1;
            }
            Err(ProtoError::Timeout) => {
                consecutive_timeouts += 1;
                let mut status = ctx.status.lock().unwrap();
                status.timeouts += 1;
                if consecutive_timeouts == SILENT_POLLS_BEFORE_ERROR {
                    status.last_error = Some("ECU not responding".into());
                    drop(status);
                    broadcast_status(&ctx);
                }
            }
            Err(e) => {
                // Io/EcuError: treat the connection as gone.
                tracing::error!("comms error, disconnecting: {e}");
                finish_log(&ctx, &mut log);
                prune_logs(&ctx);
                ctx.tune.lock().unwrap().set_loaded(false);
                let mut status = ctx.status.lock().unwrap();
                status.connected = false;
                status.tune_loaded = false;
                status.log = None;
                status.last_error = Some(e.to_string());
                drop(status);
                broadcast_status(&ctx);
                broadcast_tune(&ctx);
                return;
            }
        }
    }
}

fn prune_logs(ctx: &CommsCtx) {
    if ctx.retention_bytes == 0 {
        return;
    }
    let active = ctx
        .status
        .lock()
        .unwrap()
        .log
        .as_ref()
        .map(|l| PathBuf::from(&l.path));
    let Ok(rd) = std::fs::read_dir(&ctx.log_dir) else {
        return;
    };
    let mut files: Vec<_> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "msl") && Some(&p) != active.as_ref() {
                let m = e.metadata().ok()?;
                Some((m.modified().ok()?, m.len(), p))
            } else {
                None
            }
        })
        .collect();
    files.sort_by_key(|x| x.0);
    let mut total: u64 = files.iter().map(|x| x.1).sum();
    for (_, size, path) in files {
        if total <= ctx.retention_bytes {
            break;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => total = total.saturating_sub(size),
            Err(e) => {
                ctx.status.lock().unwrap().last_error =
                    Some(format!("log retention {}: {e}", path.display()))
            }
        }
    }
}

/// Read all pages into the tune, verifying each against the ECU's `d` CRC
/// (one retry per page).
fn download_tune(
    session: &mut Session<SerialTransport>,
    pages: &PageCommands,
    ctx: &CommsCtx,
) -> Result<(), ProtoError> {
    for page_idx in 0..pages.page_count() {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let bytes = pages.read_page(session, page_idx)?;
            let ecu_crc = pages.page_crc(session, page_idx)?;
            if ecu_crc == crc32fast::hash(&bytes) {
                ctx.tune.lock().unwrap().load_page(page_idx, &bytes);
                break;
            }
            if attempt >= 2 {
                return Err(ProtoError::ShortResponse {
                    expected: ecu_crc as usize,
                    got: crc32fast::hash(&bytes) as usize,
                });
            }
            tracing::warn!("page {page_idx} CRC mismatch on download, retrying");
        }
    }
    Ok(())
}

/// Send every dirty span with `M` writes, then verify each touched page
/// with `d`. On a verify failure the page is re-downloaded so our ECU
/// shadow matches reality and the edit goes out again next pass.
fn flush_dirty(session: &mut Session<SerialTransport>, pages: &PageCommands, ctx: &CommsCtx) {
    if !ctx.tune.lock().unwrap().loaded() {
        return;
    }
    let dirty: Vec<usize> = {
        let tune = ctx.tune.lock().unwrap();
        (0..tune.page_count())
            .filter(|&i| tune.page_dirty(i))
            .collect()
    };
    if dirty.is_empty() {
        return;
    }

    for page_idx in dirty {
        let spans = ctx
            .tune
            .lock()
            .unwrap()
            .dirty_spans(page_idx, SPAN_MERGE_GAP);
        for (offset, bytes) in spans {
            for (chunk_i, chunk) in bytes.chunks(pages.blocking_factor() as usize).enumerate() {
                let chunk_off = offset + chunk_i * pages.blocking_factor() as usize;
                match pages.write_chunk(session, page_idx, chunk_off as u16, chunk) {
                    Ok(()) => {
                        ctx.tune
                            .lock()
                            .unwrap()
                            .mark_sent(page_idx, chunk_off, chunk.len());
                    }
                    Err(e) => {
                        tracing::error!("write to page {page_idx} failed: {e}");
                        ctx.status.lock().unwrap().last_error =
                            Some(format!("write to page {} failed: {e}", page_idx + 1));
                        broadcast_status(ctx);
                        return;
                    }
                }
            }
        }

        // Verify: the ECU's page CRC must match our shadow of it.
        let expected = crc32fast::hash(&ctx.tune.lock().unwrap().page(page_idx).unwrap().ecu);
        match pages.page_crc(session, page_idx) {
            Ok(crc) if crc == expected => {}
            Ok(_) => {
                tracing::error!("page {page_idx} verify failed; resyncing");
                if let Ok(bytes) = pages.read_page(session, page_idx) {
                    ctx.tune.lock().unwrap().resync_ecu(page_idx, &bytes);
                }
                ctx.status.lock().unwrap().last_error = Some(format!(
                    "page {} write verification failed; resynced from ECU",
                    page_idx + 1
                ));
                broadcast_status(ctx);
            }
            Err(e) => {
                tracing::error!("page {page_idx} CRC check failed: {e}");
            }
        }
    }
    broadcast_tune(ctx);
}

/// Flush edits, then burn every page whose RAM differs from EEPROM.
fn do_burn(session: &mut Session<SerialTransport>, ctx: &CommsCtx) -> Result<Vec<usize>, String> {
    let Some(pages) = &ctx.pages else {
        return Err("tuning requires the primary (USB) serial".into());
    };
    if !ctx.tune.lock().unwrap().loaded() {
        return Err("tune not loaded".into());
    }
    flush_dirty(session, pages, ctx);

    let pending: Vec<usize> = {
        let tune = ctx.tune.lock().unwrap();
        (0..tune.page_count())
            .filter(|&i| tune.page_burn_pending(i))
            .collect()
    };
    let mut burned = Vec::new();
    for page_idx in pending {
        pages
            .burn(session, page_idx)
            .map_err(|e| format!("burn page {} failed: {e}", page_idx + 1))?;
        ctx.tune.lock().unwrap().mark_burned(page_idx);
        burned.push(page_idx);
    }
    Ok(burned)
}

fn start_log(ctx: &CommsCtx, path: &std::path::Path) -> std::io::Result<MslWriter> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let columns = datalog::columns(&ctx.def, ctx.defaults.as_ref());
    let title = format!(
        "\"rustytune {}\" log, {}",
        env!("CARGO_PKG_VERSION"),
        ctx.def.signature
    );
    MslWriter::create(path, &title, columns)
}

fn finish_log(ctx: &CommsCtx, log: &mut Option<MslWriter>) -> Option<LogSummary> {
    let writer = log.take()?;
    ctx.status.lock().unwrap().log = None;
    let rows = writer.rows();
    let result = match writer.finish() {
        Ok(path) => Some(LogSummary { path, rows }),
        Err(e) => {
            tracing::error!("closing datalog failed: {e}");
            None
        }
    };
    prune_logs(ctx);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> EngineShutdownConfig {
        EngineShutdownConfig {
            enabled: true,
            arm_rpm: 500.0,
            stop_rpm: 50.0,
            delay_seconds: 15,
        }
    }

    #[test]
    fn shutdown_requires_running_engine_then_continuous_stop_delay() {
        let start = Instant::now();
        let mut monitor = EngineShutdownMonitor::new(config());
        assert!(!monitor.observe(Some(0.0), start + Duration::from_secs(30)));
        assert!(!monitor.observe(Some(800.0), start + Duration::from_secs(31)));
        assert!(!monitor.observe(Some(0.0), start + Duration::from_secs(32)));
        assert!(!monitor.observe(Some(100.0), start + Duration::from_secs(40)));
        assert!(!monitor.observe(Some(0.0), start + Duration::from_secs(41)));
        assert!(monitor.observe(Some(0.0), start + Duration::from_secs(56)));
        assert!(!monitor.observe(Some(0.0), start + Duration::from_secs(57)));
    }

    #[test]
    fn missing_rpm_resets_stop_countdown() {
        let start = Instant::now();
        let mut monitor = EngineShutdownMonitor::new(config());
        assert!(!monitor.observe(Some(700.0), start));
        assert!(!monitor.observe(Some(0.0), start + Duration::from_secs(1)));
        assert!(!monitor.observe(None, start + Duration::from_secs(10)));
        assert!(!monitor.observe(Some(0.0), start + Duration::from_secs(16)));
        assert!(monitor.observe(Some(0.0), start + Duration::from_secs(31)));
    }
}
