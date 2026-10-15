#!/bin/bash
# Ubuntu-to-Ubuntu check of the *installed* package (roadmap P8-T8): two copies of
# /usr/bin/gpu-share, an owner with no graphics card and a friend. The owner downloads the
# real engine from the pinned llama.cpp release when switching sharing on, adds Qwen3 0.6B
# from Hugging Face, and answers the friend on the processor. Needs network access to
# github.com and huggingface.co, Xvfb, WebKitWebDriver and tauri-driver.
#
#   sudo apt install "./GPU Share_<version>_amd64.deb"
#   scripts/e2e/installed.sh [interface]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${OUT:-$ROOT/target/e2e-installed}"
mkdir -p "$OUT"
rm -rf "$OUT/owner" "$OUT/friend"
IFACE="${1:-}"
APP="${APP:-/usr/bin/gpu-share}"

# The engine manifest the release workflow would bundle (a local build bundles an empty one).
tag="$(tr -d '[:space:]' < "$ROOT/engine-version.txt")"
name="llama-$tag-bin-ubuntu-vulkan-x64.tar.gz"
url="https://github.com/ggml-org/llama.cpp/releases/download/$tag/$name"
if [ ! -f "$OUT/manifest.json" ]; then
  curl -sSL -o "$OUT/$name" "$url"
  jq -n --arg tag "$tag" --arg url "$url" --arg sha "$(sha256sum "$OUT/$name" | cut -d' ' -f1)" \
    --argjson size "$(stat -c %s "$OUT/$name")" '{tag: $tag, files: [{url: $url, sha256: $sha, size: $size}]}' \
    > "$OUT/manifest.json"
  rm "$OUT/$name"
fi

pids=()
cleanup() { kill "${pids[@]}" 2>/dev/null || true; }
trap cleanup EXIT

Xvfb :98 -screen 0 1280x900x24 >/dev/null 2>&1 & pids+=($!)
export DISPLAY=:98
sleep 2

common=(GPU_SHARE_IFACE="$IFACE" GPU_SHARE_ENGINE_MANIFEST="$OUT/manifest.json")
env "${common[@]}" GPU_SHARE_DATA_DIR="$OUT/owner" GPU_SHARE_NAME="Ubuntu Owner" \
  tauri-driver --port 4464 --native-port 4465 >"$OUT/owner-driver.log" 2>&1 & pids+=($!)
env "${common[@]}" GPU_SHARE_DATA_DIR="$OUT/friend" GPU_SHARE_NAME="Ubuntu Friend" GPU_SHARE_PORT=50124 \
  tauri-driver --port 4474 --native-port 4475 >"$OUT/friend-driver.log" 2>&1 & pids+=($!)
sleep 2

APP="$APP" OUT="$OUT" node "$ROOT/scripts/e2e/installed.mjs"
