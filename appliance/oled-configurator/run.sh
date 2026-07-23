#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="$SCRIPT_DIR/pi_configurator_c"

if [ ! -x "$BINARY" ]; then
    echo "[*] Building native Pi Configurator..."
    make -C "$SCRIPT_DIR"
fi

exec "$BINARY" "$@"
