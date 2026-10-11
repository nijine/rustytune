# RustyTune OLED Bonnet Configurator

This native C interface configures the RustyTune appliance on a Raspberry Pi
Zero 2 W with the Adafruit 128x64 1.3-inch OLED Bonnet. It is an independent
sub-project with no Rust or Cargo dependency. Build it directly with `make`.

An access point (AP) provides a Wi-Fi network that other devices can join.

## Features

- Direct SSD1306 framebuffer rendering over Linux I2C
- Bonnet controls through `libgpiod`
- Hierarchical Wi-Fi, System, and RustyTune menus
- Wi-Fi status, scanning, saved networks, and password entry
- Reversible client/AP mode switching
- AP network configuration that lets connected phones retain mobile internet
  access
- Editable AP SSID and WPA password with masked input and case toggle
- Persistent boot preference for AP mode, automatic client selection, or a
  specific saved client profile
- System hostname, IP address, CPU temperature, and RAM usage
- Idle dimming and display blanking
- Confirmed system shutdown from the main menu and reboot from the System menu
- RustyTune ECU, service, and logging status through its local administration
  socket, restricted to root
- RustyTune serial device, primary/secondary mode, baud, and auto-log controls
- Five-minute pairing codes, web address, storage, reconnect, and restart
  controls
- Explicit missing-installation reporting without installation or upgrade
  actions
- Optional systemd boot service

## Install and run

From this directory on Raspberry Pi OS, run:

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

To remove the boot service, run `sudo ./uninstall_service.sh`.

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
characters, and AP passwords must contain 8–63 characters. Saving configuration
does not interrupt client mode. If AP mode is active, the UI asks whether to
restart the hotspot immediately.

## Cellular data while connected to the AP

RustyTune gives its managed AP the stable address `10.0.0.1/24`. DHCP supplies
network configuration to connected devices. The NetworkManager shared-mode
dnsmasq policy omits the DHCP default-router and DNS options.

This lets a phone keep its cellular internet connection and a direct Wi-Fi
connection to the appliance. In the phone browser, open:

```text
http://10.0.0.1/
```

`setup.sh` and `install_service.sh` install the policy as
`/etc/NetworkManager/dnsmasq-shared.d/rustytune-ap.conf`. Restart AP mode or
reboot after installation so NetworkManager starts a fresh DHCP server.
Uninstalling the service removes only this RustyTune-owned fragment. Saved
client and AP profiles are not deleted.

## Faster I2C refresh

For a 400 kHz I2C bus, add the following to `/boot/firmware/config.txt` and
reboot:

```ini
dtparam=i2c_arm=on
dtparam=i2c_arm_baudrate=400000
```

## Development

The implementation is split into the SSD1306 display driver, GPIO button driver,
NetworkManager backend, RustyTune NDJSON socket client, and main UI state
machine. These components are all in the project root.

`make check` builds the program and runs a basic command-line test. RustyTune
controls require its appliance service and `/run/rustytune/admin.sock`. The
configurator does not install or upgrade that service.
