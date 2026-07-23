# RustyTune appliance

This directory contains the Linux/Raspberry Pi integration around the portable
RustyTune binary:

- `config/` — default appliance TOML configuration.
- `systemd/` — hardened RustyTune service unit.
- `oled-configurator/` — native C UI for the Adafruit OLED Bonnet.

The OLED configurator communicates only through RustyTune's root-local NDJSON
administration socket. Keeping both ends in this repository lets protocol and
UI changes ship together, while the C program retains its own Makefile and can
be built without Cargo.

Its managed Wi-Fi AP uses `10.0.0.1/24` and omits DHCP router/DNS
advertisements, allowing a connected phone to reach RustyTune while retaining
cellular internet access.

Optional engine-off shutdown is disabled by default. Once enabled, RustyTune
arms only after RPM reaches the configured running threshold. If RPM then
remains at or below the stop threshold for the configured delay, RustyTune
closes the active datalog and creates `/run/rustytune/poweroff-request`.
The root-owned `rustytune-poweroff.path` unit responds by starting the narrowly
scoped poweroff service; the network-facing RustyTune process does not receive
shutdown privileges.

The former standalone `adafruit-bonnet-pi-configurator` repository is retained
unchanged as the history and deployment rollback source. New appliance work
should be made in this sub-project.

From the repository root:

```sh
make oled
make appliance-check
```
