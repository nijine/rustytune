# rustytune

RustyTune is open-source tuning software for
[Speeduino](https://speeduino.com) ECUs and a native alternative to
TunerStudio for the day-to-day tuning loop. It connects to an ECU over serial
and provides a browser-based interface for live gauges, VE/ignition/AFR table
and settings editing, EEPROM burns, datalogging and playback, tune comparison,
and offline `.msq` editing.

A single Rust binary owns the ECU connection and serves the web UI. RustyTune
can run as a standalone desktop application on the laptop connected to the vehicle,
or as a headless Raspberry Pi appliance that can be accessed from another
device on the Pi's network.

**Status: it tunes.** Connect over USB (primary) or SER3 (secondary),
watch the INI-defined gauges live, record MegaLogViewer `.msl` datalogs,
edit VE/ignition/AFR tables in the browser (live `M` writes,
CRC-verified), edit every settings dialog the INI defines (acceleration
enrichment, idle, fan, launch, boost, ...), drag points on every
correction curve (WUE, dwell, IAT retard, idle targets, ...) with a live
operating-point cursor, burn to EEPROM, and diff the
ECU against a TunerStudio `.msq` — with selective apply and save. Recorded
logs open in the built-in Log Viewer as synced strip charts. No ECU
around? Open a `.msq` offline and edit it with the same table/settings UI,
then save it back out.

`make release` builds a single-file binary tarball for this machine;
tagging `v*` builds macOS arm64 + Linux x86_64/arm64 release artifacts in
CI (Linux arm64 covers the Raspberry Pi).

## Deployment profiles

RustyTune supports a default standalone desktop profile and an explicit
Raspberry Pi appliance profile. See
[docs/raspberry-pi-appliance.md](docs/raspberry-pi-appliance.md) for safe
installation, validation, pairing, and rollback.

Pi deployment assets live under [`appliance/`](appliance/), including the
independently buildable native OLED configurator. Run `make appliance-check`
to verify the RustyTune server and OLED companion together.

## Architecture

| Crate | Role |
|---|---|
| `ts-ini` | TunerStudio ECU-definition INI parser (Speeduino subset) |
| `ecu-proto` | Serial protocols: TunerStudio Protocol 3 envelope + secondary serial |
| `tune-model` | Page buffers, typed constant/table views, dirty tracking, `.msq` I/O |
| `datalog` | MegaLogViewer `.msl` writing/reading |
| `server` | axum server: comms thread, REST + WebSocket API, embedded web UI |

The frontend (`web/`, Vite + React + TypeScript) is built to `web/dist` and
embedded into the server binary — a release build is a single executable.

## Building

Requires Rust (stable) and Node ≥ 22.

```sh
make run        # build frontend, then cargo run (opens the browser)
make test       # fmt check + clippy + tests
make dev        # Vite dev server with HMR (run the server separately)
make bench      # hardware-free test bench (see below)
```

## Test bench — no vehicle required

`make bench` (or `tools/bench.sh`) starts a simulated Speeduino on a pty
and the server against it. The port shows up in the picker as
`/tmp/rustytune-sim`; hit Connect and everything works exactly like real
hardware: full page download with CRC verification, animated telemetry
on the gauges, table and settings edits flushed as `M` writes, burns.

The bench "EEPROM" lives in `tools/fake-ecu/bench-tune.json` (gitignored)
— burned changes survive restarting the bench, just like a real ECU
power cycle. Delete the file for a factory-fresh ECU.

Flags pass through to the simulator:

```sh
tools/bench.sh --static             # fixed reference values (RPM 3450, AFR 14.7)
tools/bench.sh --corrupt-every 50   # inject CRC errors to watch recovery
```

Because the simulator speaks the same Protocol 3 the firmware does (page
reads/writes, `d` CRC checks, burn semantics, error codes), anything
that works on the bench is exercising the identical code path used at
the vehicle — only the transport endpoint differs.

## ECU definitions

`fixtures/speeduino202501_7.ini` is the reference TunerStudio INI the
parser is developed and golden-tested against. At runtime the server loads
the INI matching your firmware, so a firmware update is a new INI file, not
a code change.

## Note on Windows builds

The project was originally intended to be cross-platform compatible in a
similar manner to the TunerStudio software that it was inspired by. However,
serial communication was originally modeled in a \*nix-only environment,
and has to be updated to support Windows. This is listed as one of the TODOs
below.

## TODOs

- Add Windows build support & matching CI workflow
- Add autotune functionality

## License

rustytune is free software: **GPL-3.0-or-later**, with one addition —
an [App Store additional permission](LICENSE-EXCEPTION.md) (GPLv3 §7)
that allows conveying builds through app stores whose terms would
otherwise conflict with the GPL, as long as full source stays publicly
available under this same license. See [LICENSE](LICENSE) and
[LICENSE-EXCEPTION.md](LICENSE-EXCEPTION.md).

Contributions are accepted under the same terms with a DCO sign-off —
see [CONTRIBUTING.md](CONTRIBUTING.md).

The reference INI in `fixtures/` originates from the
[Speeduino](https://speeduino.com) project and remains under
Speeduino's own license; it is not part of rustytune's license grant.
