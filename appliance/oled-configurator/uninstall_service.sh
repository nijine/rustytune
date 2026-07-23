#!/usr/bin/env bash
set -euo pipefail

SERVICE_NAME="adafruit-bonnet-pi-configurator.service"
TARGET_SERVICE="/etc/systemd/system/${SERVICE_NAME}"
AP_POLICY="/etc/NetworkManager/dnsmasq-shared.d/rustytune-ap.conf"

echo "======================================================"
echo " Uninstalling Pi Configurator Boot Service"
echo "======================================================"

if [ "$(id -u)" -ne 0 ]; then
    echo "[!] Error: This script must be executed with sudo."
    echo "    Usage: sudo ./uninstall_service.sh"
    exit 1
fi

echo "[1/4] Stopping and disabling service..."
if [ -f "$TARGET_SERVICE" ]; then
    systemctl stop "$SERVICE_NAME" || true
    systemctl disable "$SERVICE_NAME" || true
else
    echo "Service file ${TARGET_SERVICE} is not currently installed."
fi

echo "[2/4] Removing service file..."
rm -f "$TARGET_SERVICE"

echo "[3/4] Removing cellular-preserving AP policy..."
rm -f "$AP_POLICY"

echo "[4/4] Reloading systemd..."
systemctl daemon-reload
echo "Service uninstalled successfully."
echo "Restart NetworkManager or reboot to retire an active AP DHCP policy."
