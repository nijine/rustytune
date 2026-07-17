//! Shared harness for pty integration tests: spawns
//! tools/fake-ecu/fake_ecu.py and connects a Session to it.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ecu_proto::{Config, Mode, SerialTransport, Session};
use ts_ini::IniDef;

/// Spawn fake_ecu.py; kills the process and removes the link on drop.
pub struct FakeEcu {
    child: Child,
    pub link: PathBuf,
}

impl FakeEcu {
    pub fn spawn(mode: &str, extra_args: &[&str]) -> FakeEcu {
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

    pub fn session(&self, mode: Mode) -> Session<SerialTransport> {
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

#[allow(dead_code)]
pub fn fixture(symbols: &[&str]) -> IniDef {
    let src = include_str!("../../../../fixtures/speeduino202405_dev.ini");
    let symbols: HashSet<String> = symbols.iter().map(|s| s.to_string()).collect();
    ts_ini::parse_with_symbols(src, &symbols).unwrap()
}
