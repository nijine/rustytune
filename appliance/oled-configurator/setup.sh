#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "======================================================"
echo " Adafruit OLED Bonnet Pi Configurator Setup (Native C)"
echo "======================================================"

if command -v apt-get >/dev/null 2>&1; then
    echo "[1/4] Installing system packages..."
    sudo apt-get update
    sudo apt-get install -y \
        build-essential \
        pkg-config \
        i2c-tools \
        libgpiod-dev \
        network-manager
else
    echo "[1/4] apt-get not found; assuming build dependencies are installed."
fi

echo "[2/4] Installing cellular-preserving AP policy..."
sudo install -D -m 0644 \
    "$SCRIPT_DIR/rustytune-ap-dnsmasq.conf" \
    /etc/NetworkManager/dnsmasq-shared.d/rustytune-ap.conf

if command -v raspi-config >/dev/null 2>&1; then
    echo "[3/4] Enabling I2C..."
    sudo raspi-config nonint do_i2c 0
else
    echo "[3/4] raspi-config not found; verify I2C is enabled manually."
fi

if [ -n "${USER:-}" ]; then
    sudo usermod -aG i2c,gpio,netdev "$USER" || true
fi

echo "[4/4] Building native application..."
make -C "$SCRIPT_DIR" clean all check

echo "Setup complete. Run with: sudo ./run.sh"
echo "Install at boot with: sudo ./install_service.sh"
echo "Restart AP mode or reboot before testing the new DHCP policy."
