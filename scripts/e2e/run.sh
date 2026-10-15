#!/bin/bash
# End-to-end check of two real app instances on one Linux machine (see two-apps.mjs).
# Needs: scripts/dev-engine.sh done, Xvfb, webkit2gtk-driver (WebKitWebDriver), and
# `cargo install tauri-driver --locked`. Not part of CI (no engine there).
#
#   scripts/e2e/run.sh [interface]     # interface for multicast, e.g. eth0 (default: automatic)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${OUT:-$ROOT/target/e2e}"
mkdir -p "$OUT"
rm -rf "$OUT/owner" "$OUT/friend"
IFACE="${1:-}"

cargo build -p app --manifest-path "$ROOT/Cargo.toml"
APP="$ROOT/target/debug/app"

pids=()
cleanup() { kill "${pids[@]}" 2>/dev/null || true; }
trap cleanup EXIT

(cd "$ROOT/apps/desktop" && npx vite --port 5173 --strictPort >"$OUT/vite.log" 2>&1) & pids+=($!)
Xvfb :99 -screen 0 1280x900x24 >/dev/null 2>&1 & pids+=($!)
export DISPLAY=:99
sleep 2

# the owner shares on the default port; the friend never switches sharing on
echo '{}' > "$OUT/world.json"
GPU_SHARE_DATA_DIR="$OUT/owner" GPU_SHARE_NAME="E2E Owner" GPU_SHARE_IFACE="$IFACE" GPU_SHARE_WORLD_FILE="$OUT/world.json" \
  tauri-driver --port 4444 --native-port 4445 >"$OUT/owner-driver.log" 2>&1 & pids+=($!)
GPU_SHARE_DATA_DIR="$OUT/friend" GPU_SHARE_NAME="E2E Friend" GPU_SHARE_IFACE="$IFACE" GPU_SHARE_PORT=50124 \
  tauri-driver --port 4454 --native-port 4455 >"$OUT/friend-driver.log" 2>&1 & pids+=($!)
sleep 2

APP="$APP" OUT="$OUT" node "$ROOT/scripts/e2e/two-apps.mjs"
