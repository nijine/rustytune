//! End-to-end test: real axum server, real pty-backed fake ECU, driven the
//! way the browser drives it — REST connect, WebSocket telemetry, .msl
//! logging start/stop.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;

struct FakeEcu {
    child: Child,
    link: PathBuf,
}

impl FakeEcu {
    fn spawn() -> FakeEcu {
        let script =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/fake-ecu/fake_ecu.py");
        let link = std::env::temp_dir().join(format!("rustytune-e2e-{}", std::process::id()));
        let _ = std::fs::remove_file(&link);

        let child = Command::new("python3")
            .arg(&script)
            .args(["--mode", "primary", "--static", "--och-size", "127"])
            .args(["--link", link.to_str().unwrap()])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("python3 must be available to run the fake ECU");

        let deadline = Instant::now() + Duration::from_secs(10);
        while !link.exists() {
            assert!(Instant::now() < deadline, "fake ECU never created {link:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
        FakeEcu { child, link }
    }
}

impl Drop for FakeEcu {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.link);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn browser_workflow() {
    let ecu = FakeEcu::spawn();

    let log_dir = std::env::temp_dir().join(format!("rustytune-e2e-logs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&log_dir);

    let def = ts_ini::parse(rustytune_server::EMBEDDED_INI).unwrap();
    let state = rustytune_server::build_state(def, log_dir.clone());
    let app = rustytune_server::app(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let base = format!("http://{addr}/api");
    let http = reqwest::Client::new();

    // Definition: front page gauges resolved server-side.
    let definition: serde_json::Value = http
        .get(format!("{base}/definition"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(definition["signature"], "speeduino 202405-dev");
    assert_eq!(definition["gauges"][0]["channel"], "rpm");
    assert_eq!(definition["gauges"][0]["hi"], 8000.0);
    let n_indicators = definition["indicators"].as_array().unwrap().len();
    assert!(n_indicators > 10);

    // Ports endpoint answers (contents are machine-dependent).
    assert!(
        http.get(format!("{base}/ports"))
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );

    // Connect to the fake ECU over its pty symlink.
    let resp = http
        .post(format!("{base}/connect"))
        .json(&serde_json::json!({
            "port": ecu.link.to_str().unwrap(),
            "mode": "primary",
            "pollMs": 20,
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{:?}", resp.text().await);
    // A second connect while connected is rejected.
    let resp = http
        .post(format!("{base}/connect"))
        .json(&serde_json::json!({ "port": "/nonexistent", "mode": "primary" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409);

    // WebSocket: first message is a status snapshot, then telemetry frames
    // with the fake ECU's static reference values.
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws"))
        .await
        .unwrap();
    let mut saw_status = false;
    let mut frames = 0u32;
    let deadline = Instant::now() + Duration::from_secs(10);
    while frames < 5 {
        assert!(Instant::now() < deadline, "not enough frames over WS");
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("ws message")
            .expect("ws open")
            .expect("ws ok");
        let Message::Text(text) = msg else { continue };
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        match value["type"].as_str() {
            Some("status") => {
                saw_status = true;
                assert_eq!(value["connected"], true);
            }
            Some("frame") => {
                frames += 1;
                assert_eq!(value["channels"]["rpm"], 3450.0);
                assert_eq!(value["channels"]["tps"], 22.0);
                assert_eq!(value["channels"]["afr"], 14.7);
                let indicators = value["indicators"].as_array().unwrap();
                assert_eq!(indicators.len(), n_indicators);
            }
            Some("tune") => {}       // dirty/burn broadcasts ride the same socket
            Some("definition") => {} // gauge-limit edits re-push the definition
            other => panic!("unexpected message type {other:?}"),
        }
    }
    assert!(saw_status, "ws must open with a status snapshot");

    // The comms thread verified the ECU signature against the INI.
    let status: serde_json::Value = http
        .get(format!("{base}/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["ecuSignature"], "speeduino 202405-dev");
    assert_eq!(status["lastError"], serde_json::Value::Null);

    // Tune download: poll until every page is in and CRC-verified.
    let deadline = Instant::now() + Duration::from_secs(10);
    let tune = loop {
        let tune: serde_json::Value = http
            .get(format!("{base}/tune"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if tune["loaded"] == true {
            break tune;
        }
        assert!(Instant::now() < deadline, "tune never loaded: {tune}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(tune["dirty"], false);
    assert_eq!(tune["burnPending"], false);

    // VE table decodes from the fake ECU's pattern pages.
    let table: serde_json::Value = http
        .get(format!("{base}/tune/table/veTable1Tbl"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(table["z"].as_array().unwrap().len(), 16);
    assert_eq!(table["z"][0].as_array().unwrap().len(), 16);
    // Page 2 pattern: byte i = (2*31 + i) & 0xFF; veTable offset 0,
    // rpmBins offset 256 at scale 100.
    assert_eq!(table["z"][0][0], 62.0);
    assert_eq!(table["x"][0], 6200.0);

    // Client A edits a cell; the writer lock now belongs to A.
    let resp = http
        .post(format!("{base}/tune/table/veTable1Tbl/cells"))
        .header("X-Client-Id", "client-a")
        .json(&serde_json::json!({ "cells": [{ "row": 0, "col": 0, "value": 75.0 }] }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{:?}", resp.text().await);

    // Client B is read-only while A holds the lock.
    let resp = http
        .post(format!("{base}/tune/table/veTable1Tbl/cells"))
        .header("X-Client-Id", "client-b")
        .json(&serde_json::json!({ "cells": [{ "row": 0, "col": 1, "value": 80.0 }] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 423, "second writer must be locked out");

    // The comms thread flushes the edit with M and verifies with d.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let tune: serde_json::Value = http
            .get(format!("{base}/tune"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if tune["dirty"] == false {
            assert_eq!(tune["burnPending"], true, "flushed but not burned");
            break;
        }
        assert!(Instant::now() < deadline, "edit never flushed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let status: serde_json::Value = http
        .get(format!("{base}/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        status["lastError"],
        serde_json::Value::Null,
        "write verification must pass"
    );
    let table: serde_json::Value = http
        .get(format!("{base}/tune/table/veTable1Tbl"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(table["z"][0][0], 75.0);

    // Burn commits RAM to "EEPROM" (page index 1).
    let resp = http
        .post(format!("{base}/tune/burn"))
        .header("X-Client-Id", "client-a")
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let burned: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(burned["burnedPages"][0], 1);
    let tune: serde_json::Value = http
        .get(format!("{base}/tune"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tune["burnPending"], false);

    // Gauge limits are PcVariables: editable app-side, no burn involved,
    // and the definition (gauge bounds) re-resolves immediately.
    let dialog: serde_json::Value = http
        .get(format!("{base}/tune/dialog/gaugeLimits"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let rpmhigh = dialog["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["type"] == "constant" && i["constant"]["name"] == "rpmhigh")
        .expect("rpmhigh renders as an editable value");
    assert_eq!(rpmhigh["constant"]["value"], 8000.0);

    let resp = http
        .post(format!("{base}/tune/constant/rpmhigh"))
        .header("X-Client-Id", "client-a")
        .json(&serde_json::json!({ "value": 9000.0 }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let updated: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(updated["value"], 9000.0);
    assert_eq!(updated["requiresPowerCycle"], false);

    let definition: serde_json::Value = http
        .get(format!("{base}/definition"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        definition["gauges"][0]["hi"], 9000.0,
        "tachometer hi must track the edited rpmhigh"
    );
    // App-side write: nothing became dirty or burn-pending.
    let tune: serde_json::Value = http
        .get(format!("{base}/tune"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tune["dirty"], false);
    assert_eq!(tune["burnPending"], false);

    // Settings dialogs: the INI's [Menu] tree resolves to [UserDefined]
    // forms with live values and evaluated enable conditions.
    let menus: serde_json::Value = http
        .get(format!("{base}/tune/menus"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let startup = menus
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["title"] == "Startup/Idle")
        .expect("Startup/Idle menu");
    let idle_entry = startup["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "idleSettings")
        .expect("idleSettings entry");
    assert_eq!(idle_entry["type"], "dialog");
    assert_eq!(idle_entry["label"], "Idle Control");

    let dialog: serde_json::Value = http
        .get(format!("{base}/tune/dialog/idleSettings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(dialog["title"], "Idle Settings");
    let items = dialog["items"].as_array().unwrap();
    let algo = items
        .iter()
        .find(|i| i["type"] == "constant" && i["constant"]["name"] == "iacAlgorithm")
        .expect("iacAlgorithm field");
    assert_eq!(algo["label"], "Idle control type");
    assert!(!algo["constant"]["labels"].as_array().unwrap().is_empty());
    assert!(
        items.iter().any(|i| i["type"] == "panel"),
        "idleSettings must embed its sub-panels"
    );

    // "Crank to run taper" is enable-gated on iacAlgorithm ∈ {2,4,5,7};
    // set the algorithm through the same constants endpoint the form uses
    // and watch the flag flip.
    let taper_enabled = |dialog: &serde_json::Value| {
        dialog["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["type"] == "constant" && i["constant"]["name"] == "idleTaperTime")
            .map(|i| i["enabled"] == true)
    };
    for (algo, expect) in [(2.0, true), (1.0, false)] {
        let resp = http
            .post(format!("{base}/tune/constant/iacAlgorithm"))
            .header("X-Client-Id", "client-a")
            .json(&serde_json::json!({ "value": algo }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());
        let dialog: serde_json::Value = http
            .get(format!("{base}/tune/dialog/idleSettings"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            taper_enabled(&dialog),
            Some(expect),
            "idleTaperTime gate at iacAlgorithm={algo}"
        );
    }
    // Empty-titled dialogs fall back to their menu label.
    let dialog: serde_json::Value = http
        .get(format!("{base}/tune/dialog/engine_constants"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(dialog["title"], "Engine Constants");

    // Wait for the settings writes to flush so later dirty checks are clean.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let tune: serde_json::Value = http
            .get(format!("{base}/tune"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if tune["dirty"] == false {
            break;
        }
        assert!(Instant::now() < deadline, "settings edit never flushed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // .msq upload: the real TunerStudio file (202501) against the 202405-dev
    // definition — diff still works name-wise, mismatch is surfaced.
    let msq_content =
        tune_model::msq::decode_latin1(include_bytes!("../../../fixtures/CurrentTune.msq"));
    let meta: serde_json::Value = http
        .post(format!("{base}/msq"))
        .json(&serde_json::json!({ "filename": "CurrentTune.msq", "content": msq_content }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(meta["signature"], "speeduino 202501");
    assert_eq!(meta["signatureMatch"], false);

    let diff: serde_json::Value = http
        .get(format!("{base}/msq/diff"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let entries = diff["entries"].as_array().unwrap();
    assert!(
        !entries.is_empty(),
        "pattern pages must differ from a real tune"
    );
    let ve = entries
        .iter()
        .find(|e| e["name"] == "veTable")
        .expect("veTable differs");
    assert_eq!(ve["where"], "VE Table");
    assert!(ve["cells"][0]["row"].is_number(), "2D cells carry row/col");

    // Selective apply: push just reqFuel (7.9 in the file) to the ECU.
    let report: serde_json::Value = http
        .post(format!("{base}/msq/apply"))
        .header("X-Client-Id", "client-a")
        .json(&serde_json::json!({ "names": ["reqFuel"] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(report["applied"], 1, "{report}");
    let constants: serde_json::Value = http
        .get(format!("{base}/tune/constants?names=reqFuel"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(constants[0]["value"], 7.9);
    // reqFuel no longer appears in the diff.
    let diff: serde_json::Value = http
        .get(format!("{base}/msq/diff"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        diff["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["name"] != "reqFuel"),
        "applied constant must drop out of the diff"
    );

    // Save: the ECU state serializes as a .msq with our signature.
    let saved = http.get(format!("{base}/msq/save")).send().await.unwrap();
    assert!(saved.status().is_success());
    let disposition = saved
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(disposition.contains(".msq"), "{disposition}");
    let body = saved.text().await.unwrap();
    assert!(body.contains("signature=\"speeduino 202405-dev\""));
    let reparsed = tune_model::msq::parse(&body).unwrap();
    assert!(reparsed.constants.len() > 500);

    // Datalog: start, let some rows accumulate, stop, check the .msl.
    let start: serde_json::Value = http
        .post(format!("{base}/log/start"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let log_path = PathBuf::from(start["path"].as_str().expect("log path"));
    tokio::time::sleep(Duration::from_millis(400)).await;
    let stop = http.post(format!("{base}/log/stop")).send().await.unwrap();
    assert!(stop.status().is_success());
    let summary: serde_json::Value = stop.json().await.unwrap();
    assert!(summary["rows"].as_u64().unwrap() > 0, "{summary}");

    let msl = std::fs::read_to_string(&log_path).unwrap();
    let mut lines = msl.split("\r\n");
    assert!(lines.next().unwrap().contains("rustytune"));
    let labels: Vec<&str> = lines.next().unwrap().split('\t').collect();
    assert_eq!(labels[0], "Time");
    assert_eq!(labels[1], "SecL");
    assert_eq!(labels[2], "RPM");
    let units: Vec<&str> = lines.next().unwrap().split('\t').collect();
    assert_eq!(units.len(), labels.len());
    let first_row: Vec<&str> = lines.next().unwrap().split('\t').collect();
    assert_eq!(first_row.len(), labels.len());
    assert_eq!(first_row[2], "3450"); // RPM, "%d"

    // Disconnect tears the comms thread down.
    let resp = http
        .post(format!("{base}/disconnect"))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let status: serde_json::Value = http
        .get(format!("{base}/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["connected"], false);

    let _ = std::fs::remove_dir_all(&log_dir);
}
