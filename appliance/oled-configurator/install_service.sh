#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVICE_NAME="adafruit-bonnet-pi-configurator.service"
TARGET_SERVICE="/etc/systemd/system/$SERVICE_NAME"

if [ "$(id -u)" -ne 0 ]; then
    echo "Error: run this installer with sudo."
    echo "Usage: sudo ./install_service.sh"
    exit 1
fi

echo "[1/4] Building native application..."
make -C "$SCRIPT_DIR" clean all check

echo "[2/4] Installing cellular-preserving AP policy..."
install -D -m 0644 \
    "$SCRIPT_DIR/rustytune-ap-dnsmasq.conf" \
    /etc/NetworkManager/dnsmasq-shared.d/rustytune-ap.conf

echo "[3/4] Installing systemd service..."
sed "s|{{PROJECT_DIR}}|$SCRIPT_DIR|g" \
    "$SCRIPT_DIR/adafruit-bonnet-pi-configurator.service" > "$TARGET_SERVICE"
chmod 644 "$TARGET_SERVICE"

echo "[4/4] Enabling and starting service..."
systemctl daemon-reload
systemctl enable "$SERVICE_NAME"
systemctl restart "$SERVICE_NAME"

echo "Service installed."
echo "Status: sudo systemctl status $SERVICE_NAME"
echo "Logs:   sudo journalctl -u $SERVICE_NAME -f"
echo "Restart AP mode or reboot before testing the new DHCP policy."
