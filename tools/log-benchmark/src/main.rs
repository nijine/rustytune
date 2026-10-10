//! Run one scenario per process so /usr/bin/time can measure its peak RSS.
#[path = "../../../crates/server/src/log_view.rs"]
mod log_view;

use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3
        || !["before", "overview", "reopen", "zoom", "cached"].contains(&args[1].as_str())
    {
        return Err(
            "usage: rustytune-log-benchmark before|overview|reopen|zoom|cached FILE.msl".into(),
        );
    }
    let scenario = &args[1];
    let path = PathBuf::from(&args[2]);
    let name = path
        .file_name()
        .ok_or("missing file name")?
        .to_string_lossy()
        .into_owned();
    let mut cache = None;
    let disk_cache = path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(".rustytune-cache")
        .join(format!(
            "{}.json",
            path.file_name().unwrap().to_string_lossy()
        ));
    // Explicit cold application-cache scenario, including persistent cache writes.
    // Use copied logs in an isolated benchmark directory when running this case.
    if scenario == "overview" {
        match std::fs::remove_file(&disk_cache) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    // Zoom and cached trials start after opening the file, as they do in the UI.
    // Setup remains part of process peak RSS, but not the reported request time.
    if scenario == "zoom" || scenario == "cached" || scenario == "reopen" {
        log_view::load(
            &mut cache,
            path.clone(),
            name.clone(),
            log_view::Query::default(),
        )?;
    }
    if scenario == "reopen" {
        if !disk_cache.is_file() {
            return Err("persistent cache was not written; cannot measure reopening".into());
        }
        cache = None;
    }
    let start = Instant::now();
    let (rows, visible_rows, points, sampled, load_ms, bytes) = if scenario == "before" {
        // Original /data handler: read entire text, parse every column, serialize
        // the same JSON object it returned before progressive loading.
        let text = std::fs::read_to_string(&path)?;
        let data = datalog::read_msl(&text)?;
        drop(text);
        let rows = data.rows;
        let load_ms = start.elapsed().as_secs_f64() * 1000.0;
        let body = serde_json::json!({
            "name": name, "title": data.title, "labels": data.labels,
            "units": data.units, "rows": rows, "columns": data.columns,
        });
        let bytes = serde_json::to_vec(&body)?;
        (rows, rows, rows, false, load_ms, bytes)
    } else {
        let query = if scenario == "zoom" {
            log_view::Query {
                start: Some(100.0),
                end: Some(350.0),
                ..Default::default()
            }
        } else {
            log_view::Query::default()
        };
        let view = log_view::load(&mut cache, path.clone(), name, query)?;
        let load_ms = start.elapsed().as_secs_f64() * 1000.0;
        let bytes = serde_json::to_vec(&view)?;
        (
            view.rows,
            view.visible_rows,
            view.columns[0].len(),
            view.sampled,
            load_ms,
            bytes,
        )
    };
    let total_ms = start.elapsed().as_secs_f64() * 1000.0;
    // Keep the serialized response alive through timing and prevent elimination.
    std::hint::black_box(&bytes);
    println!(
        "{}",
        serde_json::json!({
            "scenario": scenario, "file": path.file_name().unwrap().to_string_lossy(),
            "file_bytes": std::fs::metadata(&path)?.len(), "rows": rows,
            "visible_rows": visible_rows, "points": points, "sampled": sampled,
            "load_ms": load_ms, "serialization_ms": total_ms - load_ms,
            "total_ms": total_ms, "response_bytes": bytes.len(),
            "architecture": std::env::consts::ARCH, "release": !cfg!(debug_assertions),
        })
    );
    Ok(())
}
