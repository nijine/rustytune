#!/usr/bin/env bash
# One-command hardware-free test bench: a fake Speeduino on a pty plus
# rustytune connected to it. Ctrl-C stops both.
#
#   tools/bench.sh                # animated telemetry, full tuning stack
#   tools/bench.sh --static       # fixed reference values (RPM 3450, ...)
#   tools/bench.sh --corrupt-every 50   # exercise CRC-error recovery
#
# The simulated port shows up in the UI's port picker as
# /tmp/rustytune-sim. Burns persist in tools/fake-ecu/bench-tune.json —
# the bench "EEPROM" — so your tune survives restarts; delete that file
# for a factory-fresh ECU.
set -euo pipefail
cd "$(dirname "$0")/.."

LINK=/tmp/rustytune-sim
PID_FILE=${LINK}.pid
STORAGE=tools/fake-ecu/bench-tune.json

if [ ! -f web/dist/index.html ]; then
    echo "web frontend not built yet — building..."
    (cd web && npm install && npm run build)
fi

python3 tools/fake-ecu/fake_ecu.py \
    --mode primary --och-size 127 \
    --link "$LINK" --storage "$STORAGE" "$@" &
ECU_PID=$!
printf '%s\n' "$ECU_PID" > "$PID_FILE"
trap 'kill "$ECU_PID" 2>/dev/null || true; rm -f "$LINK" "$PID_FILE"' EXIT

for _ in $(seq 50); do
    [ -e "$LINK" ] && break
    sleep 0.1
done
[ -e "$LINK" ] || { echo "fake ECU failed to start" >&2; exit 1; }

echo "fake ECU on $LINK (EEPROM: $STORAGE)"
cargo run -p rustytune-server
