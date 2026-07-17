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
            other => panic!("unexpected message type {other:?}"),
        }
    }
    assert!(saw_status, "ws must open with a status snapshot");

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
