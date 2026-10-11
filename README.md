# rustytune

RustyTune is open-source tuning software for [Speeduino](https://speeduino.com)
ECUs (engine control units). It provides a native alternative to TunerStudio for
everyday tuning.

A single Rust binary connects to the ECU over serial and serves a browser
interface. Run RustyTune on the laptop connected to the vehicle, or on a
Raspberry Pi without a display. Other devices on the Pi's network can access its
interface.

## Tuning and logs

RustyTune supports these tasks:

- Connect over USB (primary serial) or SER3 (secondary serial).
- View live gauges defined by the ECU's INI definition file.
- Edit VE, ignition, and AFR tables in the browser. Live `M` writes use CRC
  checks to make sure that the ECU receives the data correctly.
- Edit every settings dialog defined by the INI, including acceleration
  enrichment, idle, fan, launch, and boost.
- Drag points on correction curves, including WUE, dwell, IAT retard, and idle
  targets. A live cursor shows the current operating point.
- Burn changes to EEPROM, the ECU's persistent storage.
- Compare the ECU tune with a TunerStudio `.msq` file, apply selected changes,
  and save the result.
- Open a `.msq` file offline, edit its tables and settings, and save it.
- Record MegaLogViewer `.msl` datalogs and view them as synchronized strip
  charts in the built-in Log Viewer.

Use Download all logs in the Logs menu or Log Viewer to save all available
`.msl` files in one ZIP. For a complete recording, stop logging before the
download. Downloads of active recordings include only the data written so far.

### Dashboard gauges

Open Choose gauges on the Dashboard to select gauges from
`[GaugeConfigurations]`. Use the up/down buttons to change their order.
Selections apply to both Dials and Tiles.

The browser saves your selection and uses it instead of the INI's `[FrontPage]`
list. Reset to defaults restores the INI selection and order. The INI still
supplies gauge channels, labels, units, and limits.

### AFR sensor calibration

AFR is the air-fuel ratio. To calibrate the sensor, connect over primary USB
serial and open Settings → Tools → Calibrate AFR Sensor.

Read the ECU checksum to identify a known curve. Select an AEM or another
supported INI preset, or enter two voltage/AFR points with Custom Linear WB.
Make sure that the preset matches the manual for your exact controller model.

Stop the engine before you select Write to ECU. RustyTune makes sure that RPM is
0, sends the complete table, and checks the stored checksum. Calibration saves
directly to EEPROM and does not need an additional Burn.

Speeduino returns only a calibration checksum, not the full table. Curves that
do not match a known checksum appear as unknown/custom. This browser remembers
successful custom writes and shows their points when the live checksum matches.

Calibration is separate from `.msq` files and cannot be edited offline. External
`.inc` curves and thermistor calibration are not supported yet.

## Release builds

`make release` builds a tarball with a single binary for this machine. Tags that
match `v*` trigger automated release builds for macOS arm64 and Linux
x86_64/arm64. Linux arm64 covers the Raspberry Pi.

Local and tagged release builds show the source commit's short Git SHA in the
web interface footer. This identifier tells you which source version built the
release.

## Deployment profiles

RustyTune supports a default standalone desktop profile and an explicit
Raspberry Pi appliance profile. See
[docs/raspberry-pi-appliance.md](docs/raspberry-pi-appliance.md) for
installation, checks, pairing, and rollback.

Pi deployment assets live under [`appliance/`](appliance/), including the
independently buildable native OLED configurator. Run `make appliance-check` to
build and check the RustyTune server and OLED companion together.

## Architecture

| Crate | Role |
|---|---|
| `ts-ini` | TunerStudio ECU-definition INI parser (Speeduino subset) |
| `ecu-proto` | Serial protocols: TunerStudio Protocol 3 envelope + secondary serial |
| `tune-model` | Page buffers, typed constant/table views, dirty tracking, `.msq` I/O |
| `datalog` | MegaLogViewer `.msl` writing/reading |
| `server` | axum server: comms thread, REST + WebSocket API, embedded web UI |
| `desktop` | Tauri shell: native window around an in-process server |

The frontend (`web/`, Vite + React + TypeScript) is built to `web/dist` and
embedded into the server binary. A release build is a single executable.

## Building

Install stable Rust and Node 22 or later before building.

```sh
make run        # build frontend, then cargo run (opens the browser)
make desktop-run # build and run the native Tauri desktop app
make test       # fmt check + clippy + tests
make dev        # Vite dev server with HMR (run the server separately)
make bench      # hardware-free test bench (see below)
```

The native desktop build uses Tauri 2 and keeps the browser/server build
available alongside it. Install the bundler with `cargo install tauri-cli
--version '^2.0.0' --locked`, then run `make desktop-build` to produce the
platform bundle. On Debian/Ubuntu, desktop development additionally requires:

```sh
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

On macOS, the bundle target creates the DMG directly with `hdiutil`, avoiding
Finder/AppleScript automation while retaining the app and Applications shortcut.
Other platforms use Tauri's normal bundler.

Desktop datalogs are stored below the OS-specific RustyTune application-data
directory. The native app currently targets macOS arm64 and Linux x86_64;
Windows remains blocked on the serial transport work described below.

## Test bench without a vehicle

`make bench` (or `tools/bench.sh`) starts a simulated Speeduino and connects it
to the server through a pseudo-terminal (a virtual serial connection). Select
`/tmp/rustytune-sim` in the port picker, then select Connect.

The bench supports the same operations as real hardware:

- Download complete pages with CRC checks.
- View animated telemetry on the gauges.
- Send table and settings edits as `M` writes.
- Burn changes to simulated EEPROM.

The bench stores its simulated EEPROM in `tools/fake-ecu/bench-tune.json`
(gitignored). Burned changes survive bench restarts, like changes on a real ECU
after a power cycle. To reset the simulated ECU to its factory state, delete
this file.

Flags pass through to the simulator:

```sh
tools/bench.sh --static             # fixed reference values (RPM 3450, AFR 14.7)
tools/bench.sh --corrupt-every 50   # inject CRC errors to watch recovery
```

The simulator uses the same Protocol 3 as the firmware. It supports page reads
and writes, `d` CRC checks, burns, and error codes. Bench operations use the
same code paths as operations at the vehicle, with a different connection
endpoint.

## ECU definitions

`fixtures/speeduino202501_7.ini` is the reference TunerStudio INI the parser is
developed and golden-tested against. At runtime the server loads the INI
matching your firmware, so a firmware update is a new INI file, not a code
change.

## Note on Windows builds

RustyTune was originally intended to run across platforms, like TunerStudio. Its
serial communication code currently supports Unix-like systems and needs changes
to support Windows. This work is listed in [TODOS.md](TODOS.md).

## TODOs

See [TODOS.md](TODOS.md) for known gaps and planned work. It includes the
INI-coverage audit of `fixtures/speeduino202501_7.ini`.

## License

rustytune is free software licensed under the standard GNU General Public
License version 3 or later (GPL-3.0-or-later). See [LICENSE](LICENSE) for the
full license text.

Contributions use the same terms. See [CONTRIBUTING.md](CONTRIBUTING.md).

The original license included a section 7 exemption for potential App Store
publishing. The project removed this exemption to support the Speeduino
community and keep RustyTune freely available under standard GPLv3 terms.

The reference INI in `fixtures/` originates from the
[Speeduino](https://speeduino.com) project and remains under Speeduino's own
license; it is not part of rustytune's license grant.
