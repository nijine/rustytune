//! Integration tests against the pty-backed fake Speeduino
//! (tools/fake-ecu/fake_ecu.py) — the same request/response discipline the
//! real hardware will see, minus the wiring.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ecu_proto::{Config, Mode, ProtoError, SerialTransport, Session};
use ts_ini::{IniDef, Telemetry, Value};

/// Spawn fake_ecu.py; kills the process and removes the link on drop.
struct FakeEcu {
    child: Child,
    link: PathBuf,
}

impl FakeEcu {
    fn spawn(mode: &str, extra_args: &[&str]) -> FakeEcu {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let script =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/fake-ecu/fake_ecu.py");
        let link =
            std::env::temp_dir().join(format!("rustytune-fakeecu-{}-{seq}", std::process::id()));
        let _ = std::fs::remove_file(&link);

        let child = Command::new("python3")
            .arg(&script)
            .args(["--mode", mode, "--static", "--och-size", "127"])
            .args(["--link", link.to_str().unwrap()])
            .args(extra_args)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("python3 must be available to run the fake ECU");

        // Wait for the pty symlink to appear.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !link.exists() {
            assert!(Instant::now() < deadline, "fake ECU never created {link:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
        FakeEcu { child, link }
    }

    fn session(&self, mode: Mode) -> Session<SerialTransport> {
        let transport =
            SerialTransport::open(self.link.to_str().unwrap(), 115_200).expect("open fake ECU pty");
        Session::new(transport, Config::new(mode, r"r\$tsCanId\x30%2o%2c", 127)).unwrap()
    }
}

impl Drop for FakeEcu {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.link);
    }
}

fn fixture_celsius() -> IniDef {
    let src = include_str!("../../../fixtures/speeduino202405_dev.ini");
    let symbols: HashSet<String> = ["CELSIUS".to_string()].into();
    ts_ini::parse_with_symbols(src, &symbols).unwrap()
}

#[track_caller]
fn assert_channel(telemetry: &Telemetry, name: &str, expected: f64) {
    match telemetry.channel(name) {
        Some(Value::Num(n)) => {
            assert!((n - expected).abs() < 1e-9, "{name}: {n} != {expected}");
        }
        other => panic!("channel {name}: expected number, got {other:?}"),
    }
}

/// The --static reference block, decoded through the real INI definitions.
fn assert_reference_values(def: &IniDef, block: &[u8]) {
    assert_eq!(block.len(), 127);
    let t = Telemetry::new(def, block);
    assert_channel(&t, "rpm", 3450.0);
    assert_channel(&t, "map", 98.0);
    assert_channel(&t, "tps", 22.0); // raw 44 at scale 0.5
    assert_channel(&t, "batteryVoltage", 13.9);
    assert_channel(&t, "afr", 14.7);
    assert_channel(&t, "advance", 18.0);
    // Derived channels (CELSIUS profile): raw - 40.
    assert_channel(&t, "coolant", 90.0);
    assert_channel(&t, "iat", 25.0);
}

#[test]
fn secondary_static_reference_values() {
    let ecu = FakeEcu::spawn("secondary", &[]);
    let mut session = ecu.session(Mode::Secondary);
    let def = fixture_celsius();

    for _ in 0..3 {
        let block = session.read_realtime().expect("realtime read");
        assert_reference_values(&def, &block);
    }
}

#[test]
fn primary_static_reference_values() {
    let ecu = FakeEcu::spawn("primary", &[]);
    let mut session = ecu.session(Mode::Primary);
    let def = fixture_celsius();

    for _ in 0..3 {
        let block = session.read_realtime().expect("realtime read");
        assert_reference_values(&def, &block);
    }
}

#[test]
fn primary_recovers_from_crc_corruption() {
    let ecu = FakeEcu::spawn("primary", &["--corrupt-every", "3"]);
    let mut session = ecu.session(Mode::Primary);
    let def = fixture_celsius();

    let mut ok = 0;
    let mut crc_errors = 0;
    for _ in 0..9 {
        match session.read_realtime() {
            Ok(block) => {
                assert_reference_values(&def, &block);
                ok += 1;
            }
            Err(ProtoError::CrcMismatch) => crc_errors += 1,
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
    assert_eq!(crc_errors, 3, "every 3rd response is corrupted");
    assert_eq!(ok, 6, "good frames must keep decoding after corruption");
}
