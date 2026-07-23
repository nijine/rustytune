# RustyTune OLED Bonnet Configurator

A native C appliance configuration interface for RustyTune on Raspberry Pi
Zero 2 W and the Adafruit 128x64 1.3-inch OLED Bonnet. It is maintained as an
independent sub-project: it has no Rust or Cargo dependency and can still be
built directly with `make`.

## Features

- Direct SSD1306 framebuffer rendering over Linux I2C
- Bonnet controls through `libgpiod`
- Hierarchical Wi-Fi, System, and RustyTune menus
- Wi-Fi status, scanning, saved networks, and password entry
- Reversible client/AP mode switching
- Cellular-preserving AP DHCP: connected phones retain mobile internet access
- Editable AP SSID and WPA password with masked input and case toggle
- Persistent boot preference for AP mode, automatic client selection, or a
  specific saved client profile
- System hostname, IP address, CPU temperature, and RAM usage
- Idle dimming and display blanking
- Confirmed system shutdown from the main menu and reboot from the System menu
- RustyTune ECU/service/logging status over its root-local administration socket
- RustyTune serial device, primary/secondary mode, baud, and auto-log controls
- Five-minute pairing codes, web address, storage, reconnect, and restart controls
- Explicit missing-installation reporting without installation or upgrade actions
- Optional systemd boot service

## Install and run

On Raspberry Pi OS:

```sh
./setup.sh
sudo ./run.sh
```

The setup script installs the compiler, `libgpiod`, I2C tools, and
NetworkManager before building `pi_configurator_c` in the project root.

Runtime options:

```text
--address 0x3c       OLED I2C address
--dev /dev/i2c-1     Linux I2C device
--interface wlan0    Wi-Fi interface
--idle-dim SEC       Dim timeout; 0 disables it
--idle-blank SEC     Display-off timeout; 0 disables it
```

## Boot service

```sh
sudo ./install_service.sh
sudo systemctl status adafruit-bonnet-pi-configurator
sudo journalctl -u adafruit-bonnet-pi-configurator -f
```

Remove it with `sudo ./uninstall_service.sh`.

## Controls

| Control | BCM GPIO | Menu | Keyboard |
| --- | ---: | --- | --- |
| Joystick up | 17 | Previous item | Move up |
| Joystick down | 22 | Next item | Move down |
| Joystick left | 27 | Back | Move left |
| Joystick right | 23 | — | Move right |
| Joystick center | 4 | Select | Enter key |
| Button #5 | 5 | Select/confirm | Enter key |
| Button #6 | 6 | Back/cancel | Cancel |

In a text editor, select `^` to toggle letter case. AP SSIDs must contain 1–32
characters, and AP passwords must contain 8–63 characters. Saving settings does
not interrupt client mode; if AP mode is active, the UI asks whether to restart
the hotspot immediately.

## Cellular data while connected to the AP

RustyTune gives its managed AP the stable address `10.0.0.1/24`. Its
NetworkManager shared-mode dnsmasq policy omits the DHCP default-router and DNS
options, so a phone retains its cellular route while keeping a direct Wi-Fi
route to the appliance. Open:

```text
http://10.0.0.1/
```

`setup.sh` and `install_service.sh` install the policy as
`/etc/NetworkManager/dnsmasq-shared.d/rustytune-ap.conf`. Restart AP mode or
reboot after installation so NetworkManager starts a fresh DHCP server.
Uninstalling the service removes only this RustyTune-owned fragment; saved
client and AP profiles are not deleted.

## Faster I2C refresh

For a 400 kHz I2C bus, add the following to `/boot/firmware/config.txt` and
reboot:

```ini
dtparam=i2c_arm=on
dtparam=i2c_arm_baudrate=400000
```

The implementation is split into the SSD1306 display driver, GPIO button
driver, NetworkManager backend, RustyTune NDJSON socket client, and main UI
state machine, all in the project root. `make check` performs a build and
command-line smoke test. RustyTune controls require its appliance service and
`/run/rustytune/admin.sock`; the configurator never installs or upgrades it.
