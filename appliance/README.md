# RustyTune appliance

The RustyTune appliance turns a Raspberry Pi into a dedicated interface for a
Speeduino ECU (engine control unit). The Pi connects to the ECU over serial,
runs the RustyTune server, and records datalogs.

A phone, tablet, or laptop on the vehicle's local network can access the browser
interface. Use it to view live gauges, tune the ECU, and manage logs. The Pi
keeps the serial connection to the ECU.

The appliance is designed for installation in a vehicle without a display. It
starts RustyTune automatically and reconnects to the ECU when needed. If another
network is unavailable, it can provide a Wi-Fi access point (AP).

An optional Adafruit OLED Bonnet adds controls for network setup, RustyTune
status and configuration, pairing, and basic system administration. Pairing
protects browser access. Local services handle privileged operations.

## Directory contents

This directory contains the Linux/Raspberry Pi integration around the portable
RustyTune binary:

- `config/`: default appliance TOML configuration.
- `systemd/`: hardened RustyTune service unit.
- `oled-configurator/`: native C UI for the Adafruit OLED Bonnet.

The OLED configurator communicates only through RustyTune's local administration
socket, which is restricted to root. It uses NDJSON (newline-delimited JSON).
Keeping both programs here lets protocol and UI changes ship together. The C
program has its own Makefile and builds without Cargo.

## Wi-Fi access point

The managed Wi-Fi AP uses `10.0.0.1/24` and omits DHCP router/DNS
advertisements. A connected phone can reach RustyTune and retain cellular
internet access.

## Engine-off shutdown

Optional engine-off shutdown is disabled by default. Once enabled, RustyTune
arms only after RPM reaches the configured running threshold. If RPM then
remains at or below the stop threshold for the configured delay, RustyTune
closes the active datalog and creates `/run/rustytune/poweroff-request`. The
root-owned `rustytune-poweroff.path` unit responds by starting the narrowly
scoped poweroff service. The network-facing RustyTune process does not receive
shutdown privileges.

## Development and builds

The former standalone `adafruit-bonnet-pi-configurator` repository is retained
unchanged as the history and deployment rollback source. Make new appliance
changes in this sub-project.

From the repository root:

```sh
make oled
make appliance-check
```
