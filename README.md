# rustytune

Open-source tuning software for [Speeduino](https://speeduino.com) ECUs — a
native, no-license alternative to TunerStudio for the day-to-day tuning loop:
realtime gauges, VE/ignition/AFR table editing, burn, and datalogging.

One Rust binary owns the ECU serial port and serves a browser UI on
`127.0.0.1`. Run it on the laptop next to the car over USB serial, or (later)
headless on an in-car Raspberry Pi and tune from any device on its network.

**Status: it tunes.** Connect over USB (primary) or SER3 (secondary),
watch the INI-defined gauges live, record MegaLogViewer `.msl` datalogs,
edit VE/ignition/AFR tables in the browser (live `M` writes,
CRC-verified), edit every settings dialog the INI defines (acceleration
enrichment, idle, fan, launch, boost, ...), drag points on every
correction curve (WUE, dwell, IAT retard, idle targets, ...) with a live
operating-point cursor, burn to EEPROM, and diff the
ECU against a TunerStudio `.msq` — with selective apply and save. No ECU
around? Open a `.msq` offline and edit it with the same table/settings UI,
then save it back out.

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

## Test bench — no car required

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
the car — only the transport endpoint differs.

## ECU definitions

`fixtures/speeduino202405_dev.ini` is the reference TunerStudio INI the
parser is developed and golden-tested against. At runtime the server loads
the INI matching your firmware, so a firmware update is a new INI file, not
a code change.
