# Raspberry Pi appliance deployment

The regular `rustytune` command remains the desktop application: it listens on
`127.0.0.1:8642`, opens a browser, connects and logs only when requested, and
uses `./logs`. No machine configuration or elevated privileges are required.

## Build environment

Prefer building deployment artifacts in Docker when a local Docker engine is
available. For Raspberry Pi releases, use an ARM64 Debian Bookworm container so
the resulting binary is compatible with the Pi's glibc version. Build the web
frontend before the Rust release because `rust-embed` includes `web/dist` in
the executable. Preflight the resulting binary on the Pi with `ldd` and
`rustytune --version` before replacing the installed service binary.

For an already-provisioned appliance, build and deploy in one command:

```sh
tools/deploy-pi.sh pi@rustytune.local
```

This builds the frontend and Rust server in a Debian Bookworm Linux ARM64
Docker image, uploads the resulting binary over SSH, runs the preflight checks,
and restarts `rustytune.service`. The remote user needs sudo access. If the new
service does not become active, the script restores the previous binary and
restarts it. The script updates only the application binary; initial user,
configuration, and systemd-unit provisioning still follow the steps below.

## Install without disturbing the legacy services

1. Build/test the Linux arm64 release and install it as `/usr/local/bin/rustytune`.
2. Create the `rustytune` system user and group, adding it to `dialout`.
3. Install `appliance/config/rustytune.toml` at `/etc/rustytune/rustytune.toml` and
   the units under `appliance/systemd/` in `/etc/systemd/system/`. Enable
   `rustytune.service` and `rustytune-poweroff.path`. Build and install the
   companion in `appliance/oled-configurator`. Keep both legacy repositories
   and configurations untouched.
4. Start RustyTune manually first:
   `rustytune --profile appliance --config /etc/rustytune/rustytune.toml`.
   Use the local admin socket's `{"command":"pair"}` request to obtain the
   five-minute pairing code.
5. Confirm browser pairing, telemetry, ignition-off recovery, automatic `.msl`
   creation in `/var/log/speeduino`, and reconnect after unplugging serial.
6. Only after validation, stop and disable `speeduino-logger.service` and
   `speeduino-dash.service`, then enable `rustytune.service`.

RustyTune reuses `/var/log/speeduino`; existing `.msl` logs are immediately
visible and are not modified except when the configured retention limit requires
oldest-first pruning. The active log is never a pruning candidate.

## Rollback

Stop and disable `rustytune.service`, then enable and start
`speeduino-logger.service` and `speeduino-dash.service`. No legacy installed
files, repositories, configurations, or logs need to be restored because the
migration deliberately leaves them intact.

Pairing controls application access over HTTP; it does not encrypt traffic.
Use a trusted vehicle LAN. HTTPS is outside the current deployment profile.

## Engine-off shutdown

The `[engine_shutdown]` section is disabled by default. When enabled, the
engine must first reach `arm_rpm`; this prevents shutdown while the ignition is
on before the engine starts. RPM must then remain at or below `stop_rpm` for
`delay_seconds`. Missing telemetry and RPM above `stop_rpm` cancel an active
countdown.

The comms thread finishes the `.msl` file before writing
`/run/rustytune/poweroff-request`. The systemd path unit performs the privileged
poweroff. Keep `rustytune-poweroff.path` enabled whenever this setting is used.
