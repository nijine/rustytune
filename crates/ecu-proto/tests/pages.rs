//! Page read/write/burn against the pty-backed fake ECU, driven end to end
//! by the fixture INI's command templates.

mod common;

use common::{FakeEcu, fixture};
use ecu_proto::{Mode, PageCommands, PagesConfig, ProtoError};
use ts_ini::IniDef;

fn page_commands(def: &IniDef) -> PageCommands {
    PageCommands::new(&PagesConfig {
        identifiers: &def.header.page_identifiers,
        page_read: &def.header.page_read_command,
        chunk_write: &def.header.page_chunk_write,
        crc_check: &def.header.crc32_check_command,
        burn: &def.header.burn_command,
        sizes: &def.header.page_sizes,
        blocking_factor: def.header.blocking_factor.unwrap_or(251) as u16,
        can_id: 0,
    })
    .expect("fixture page commands parse")
}

/// The fake ECU's deterministic default page content: byte i of 1-based
/// page n is (n * 31 + i) & 0xFF.
fn default_page(page_num: usize, size: u32) -> Vec<u8> {
    (0..size as usize)
        .map(|i| ((page_num * 31 + i) & 0xFF) as u8)
        .collect()
}

#[test]
fn signature_version_and_comms_check() {
    let ecu = FakeEcu::spawn("primary", &[]);
    let mut session = ecu.session(Mode::Primary);
    let def = fixture(&[]);

    // queryCommand = "Q" answers the INI signature.
    let signature = session.query_string(&def.query_command).unwrap();
    assert_eq!(signature, def.signature);
    assert_eq!(signature, "speeduino 202405-dev");

    // versionInfo = "S" answers the display string.
    let product = session.query_string(&def.version_info).unwrap();
    assert!(product.starts_with("Speeduino"), "{product}");

    // 'C' comms test: RC_OK + 0xFF, like the firmware.
    let data = session.request(b"C", &[0x00]).unwrap();
    assert_eq!(data, [0xFF]);
}

#[test]
fn read_pages_full_and_chunked() {
    let ecu = FakeEcu::spawn("primary", &[]);
    let mut session = ecu.session(Mode::Primary);
    let def = fixture(&[]);
    let pages = page_commands(&def);

    assert_eq!(pages.page_count(), 15);

    // Page index 7 (1-based page 8) is 384 bytes — larger than the
    // blockingFactor of 251, so this exercises the chunked read.
    assert_eq!(pages.page_size(7), Some(384));
    let page8 = pages.read_page(&mut session, 7).unwrap();
    assert_eq!(page8, default_page(8, 384));

    let page1 = pages.read_page(&mut session, 0).unwrap();
    assert_eq!(page1, default_page(1, 128));
}

#[test]
fn write_read_back_and_crc_agree() {
    let ecu = FakeEcu::spawn("primary", &[]);
    let mut session = ecu.session(Mode::Primary);
    let def = fixture(&[]);
    let pages = page_commands(&def);

    // Mirror the write into a local expectation (page index 1 = page 2,
    // the VE table page).
    let mut expected = default_page(2, pages.page_size(1).unwrap());
    let patch = [0xDE, 0xAD, 0xBE, 0xEF];
    expected[5..9].copy_from_slice(&patch);

    pages.write_chunk(&mut session, 1, 5, &patch).unwrap();
    assert_eq!(pages.read_page(&mut session, 1).unwrap(), expected);

    // 'd' CRC verifies the write without a re-download.
    assert_eq!(
        pages.page_crc(&mut session, 1).unwrap(),
        crc32fast::hash(&expected)
    );

    // Out-of-range writes answer SERIAL_RC_RANGE_ERR (0x84).
    let result = pages.write_chunk(&mut session, 0, 126, &patch);
    assert!(
        matches!(result, Err(ProtoError::EcuError(0x84))),
        "{result:?}"
    );
}

#[test]
fn burn_persists_across_restart() {
    let storage = std::env::temp_dir().join(format!("rustytune-burn-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&storage);
    let storage_arg = storage.to_str().unwrap().to_string();
    let def = fixture(&[]);
    let pages = page_commands(&def);

    {
        let ecu = FakeEcu::spawn("primary", &["--storage", &storage_arg]);
        let mut session = ecu.session(Mode::Primary);
        // Page 3 (index 2): write and burn. Page 4 (index 3): write only.
        pages
            .write_chunk(&mut session, 2, 0, &[0x11, 0x22])
            .unwrap();
        pages.burn(&mut session, 2).unwrap();
        pages
            .write_chunk(&mut session, 3, 0, &[0x33, 0x44])
            .unwrap();
    } // simulator killed — "power cycle"

    let ecu = FakeEcu::spawn("primary", &["--storage", &storage_arg]);
    let mut session = ecu.session(Mode::Primary);

    let mut expected3 = default_page(3, pages.page_size(2).unwrap());
    expected3[..2].copy_from_slice(&[0x11, 0x22]);
    assert_eq!(
        pages.read_page(&mut session, 2).unwrap(),
        expected3,
        "burned page survives the restart"
    );
    assert_eq!(
        pages.read_page(&mut session, 3).unwrap(),
        default_page(4, pages.page_size(3).unwrap()),
        "unburned write is lost on power cycle, like RAM"
    );

    let _ = std::fs::remove_file(&storage);
}
