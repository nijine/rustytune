//! Real HTTP/serial regression coverage for calibration writes and safeguards.
use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn afr_calibration_workflow() {
    for rpm in [3450, 0] {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("ecu");
        let storage = dir.path().join("eeprom.json");
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/fake-ecu/fake_ecu.py");
        let _ecu = Process(
            Command::new("python3")
                .arg(script)
                .args(["--mode", "primary", "--static", "--rpm", &rpm.to_string()])
                .arg("--link")
                .arg(&link)
                .arg("--storage")
                .arg(&storage)
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !link.exists() {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let def = ts_ini::parse(rustytune_server::EMBEDDED_INI).unwrap();
        let state = rustytune_server::build_state(def, dir.path().join("logs"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/api", listener.local_addr().unwrap());
        let server =
            tokio::spawn(axum::serve(listener, rustytune_server::app(state)).into_future());
        let http = reqwest::Client::new();
        let disconnected = http
            .get(format!("{base}/tune/afr-calibration"))
            .send()
            .await
            .unwrap();
        assert_eq!(disconnected.status(), 409);
        assert!(
            http.post(format!("{base}/connect"))
                .json(&serde_json::json!({"port":link,"mode":"primary","pollMs":20}))
                .send()
                .await
                .unwrap()
                .status()
                .is_success()
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status: serde_json::Value = http
                .get(format!("{base}/status"))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if status["tuneLoaded"] == true {
                break;
            }
            assert!(Instant::now() < deadline, "{status}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let before: serde_json::Value = http
            .get(format!("{base}/tune/afr-calibration"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(
            before["matchingPresets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p == "AEM Classic 30-4110 (0–5 V / 10–20 AFR)")
        );
        let menus: serde_json::Value = http
            .get(format!("{base}/tune/menus"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(menus.to_string().contains("\"type\":\"calibration\""));
        let curve = serde_json::json!({"voltsLow":0.5,"afrLow":8.5,"voltsHigh":4.5,"afrHigh":18.0,"preset":"AEM X-Series 30-0300"});
        let response = http
            .post(format!("{base}/tune/afr-calibration"))
            .header("x-client-id", "A")
            .json(&curve)
            .send()
            .await
            .unwrap();
        if rpm > 0 {
            assert_eq!(response.status(), 409);
            assert!(response.text().await.unwrap().contains("Stop the engine"));
            assert!(
                !storage.exists(),
                "rejected calibration must not write EEPROM"
            );
        } else {
            assert!(
                response.status().is_success(),
                "{}",
                response.text().await.unwrap()
            );
            let after: serde_json::Value = http
                .get(format!("{base}/tune/afr-calibration"))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_ne!(before["crc"], after["crc"]);
            assert!(
                after["matchingPresets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|p| p == "AEM X-Series 30-0300")
            );
            let saved: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&storage).unwrap()).unwrap();
            assert_eq!(saved["afrCalibration"].as_str().unwrap().len(), 2048);
            let response = http
                .post(format!("{base}/tune/afr-calibration"))
                .header("x-client-id", "B")
                .json(&curve)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 423);
            let custom =
                serde_json::json!({"voltsLow":0,"afrLow":9.1,"voltsHigh":5,"afrHigh":21.7});
            let response = http
                .post(format!("{base}/tune/afr-calibration"))
                .header("x-client-id", "A")
                .json(&custom)
                .send()
                .await
                .unwrap();
            assert!(
                response.status().is_success(),
                "{}",
                response.text().await.unwrap()
            );
            let custom_result: serde_json::Value = response.json().await.unwrap();
            assert!(
                custom_result["matchingPresets"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            let invalid = serde_json::json!({"voltsLow":2,"afrLow":10,"voltsHigh":2,"afrHigh":20});
            let response = http
                .post(format!("{base}/tune/afr-calibration"))
                .header("x-client-id", "A")
                .json(&invalid)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 400);
            let final_read: serde_json::Value = http
                .get(format!("{base}/tune/afr-calibration"))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(final_read["crc"], custom_result["crc"]);
        }
        http.post(format!("{base}/disconnect"))
            .send()
            .await
            .unwrap();
        server.abort();
    }
}
use std::future::IntoFuture;
