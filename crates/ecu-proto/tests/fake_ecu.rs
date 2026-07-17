//! Integration tests against the pty-backed fake Speeduino
//! (tools/fake-ecu/fake_ecu.py) — the same request/response discipline the
//! real hardware will see, minus the wiring.

mod common;

use common::{FakeEcu, fixture};
use ecu_proto::{Mode, ProtoError};
use ts_ini::{IniDef, Telemetry, Value};

fn fixture_celsius() -> IniDef {
    fixture(&["CELSIUS"])
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
