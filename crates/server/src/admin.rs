//! Linux-local newline-delimited JSON management socket used by the OLED
//! configurator. The containing runtime directory and socket mode keep this
//! interface local to root/the service group.
#[cfg(unix)]
pub fn spawn(state: crate::api::SharedState, path: std::path::PathBuf) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?
    }
    if path.exists() {
        std::fs::remove_file(&path)?
    }
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660))?;
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let s = state.clone();
            tokio::spawn(handle(s, stream));
        }
    });
    Ok(())
}
#[cfg(unix)]
async fn handle(state: crate::api::SharedState, stream: tokio::net::UnixStream) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let request: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _ = write
                    .write_all(format!("{{\"error\":\"invalid JSON: {e}\"}}\n").as_bytes())
                    .await;
                continue;
            }
        };
        let response = match request.get("command").and_then(|v| v.as_str()) {
            Some("status") => serde_json::to_value(state.status.lock().unwrap().clone()).unwrap(),
            Some("config") => serde_json::to_value(&*state.runtime).unwrap(),
            Some("pair") => match state.auth.open_pairing() {
                Ok(v) => serde_json::to_value(v).unwrap(),
                Err(e) => serde_json::json!({"error":e}),
            },
            Some("reconnect") => {
                if let Some(h) = state.comms.lock().unwrap().take() {
                    let _ = h.cmd_tx.send(crate::comms::Cmd::Shutdown);
                    let _ = h.join.join();
                }
                serde_json::json!({"ok":true})
            }
            Some("configure") => {
                let Some(path) = state.runtime.source_path.as_ref() else {
                    let response = serde_json::json!({"error":"no appliance configuration file"});
                    let _ = write.write_all(format!("{response}\n").as_bytes()).await;
                    continue;
                };
                let mut cfg = (*state.runtime).clone();
                if let Some(v) = request.get("device").and_then(|v| v.as_str()) {
                    cfg.ecu.device = v.to_owned()
                }
                if let Some(v) = request.get("mode").and_then(|v| v.as_str()) {
                    if matches!(v, "primary" | "secondary") {
                        cfg.ecu.mode = v.to_owned()
                    }
                }
                if let Some(v) = request.get("baud").and_then(|v| v.as_u64()) {
                    if (1_200..=1_000_000).contains(&v) {
                        cfg.ecu.baud = v as u32
                    }
                }
                if let Some(v) = request.get("autoLog").and_then(|v| v.as_bool()) {
                    cfg.logging.auto = v
                }
                match toml::to_string_pretty(&cfg) {
                    Ok(text) => match std::fs::write(path, text) {
                        Ok(()) => serde_json::json!({"ok":true,"restartRequired":true}),
                        Err(e) => serde_json::json!({"error":format!("save configuration: {e}")}),
                    },
                    Err(e) => serde_json::json!({"error":format!("serialize configuration: {e}")}),
                }
            }
            Some("restart") => {
                let restart_state = state.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Some(handle) = restart_state.comms.lock().unwrap().take() {
                            let _ = handle.cmd_tx.send(crate::comms::Cmd::Shutdown);
                            let _ = handle.join.join();
                        }
                    })
                    .await;
                    std::process::exit(75);
                });
                serde_json::json!({"ok":true})
            }
            _ => serde_json::json!({"error":"unknown command"}),
        };
        let _ = write.write_all(format!("{response}\n").as_bytes()).await;
    }
}
#[cfg(not(unix))]
pub fn spawn(_: crate::api::SharedState, _: std::path::PathBuf) -> std::io::Result<()> {
    Ok(())
}
