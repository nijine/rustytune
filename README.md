# rustytune

Open-source tuning software for [Speeduino](https://speeduino.com) ECUs — a
native, no-license alternative to TunerStudio for the day-to-day tuning loop:
realtime gauges, VE/ignition/AFR table editing, burn, and datalogging.

One Rust binary owns the ECU serial port and serves a browser UI on
`127.0.0.1`. Run it on the laptop next to the car over USB serial, or (later)
headless on an in-car Raspberry Pi and tune from any device on its network.

**Status: early scaffold.** Nothing tunes anything yet.

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
```

## ECU definitions

`fixtures/speeduino202405_dev.ini` is the reference TunerStudio INI the
parser is developed and golden-tested against. At runtime the server loads
the INI matching your firmware, so a firmware update is a new INI file, not
a code change.
