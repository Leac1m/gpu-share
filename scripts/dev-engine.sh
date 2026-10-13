#!/bin/bash
# Builds a CPU-only llama-server at the pinned llama.cpp tag and fetches a tiny
# GGUF model, for developing Phase 2 on a machine with no GPU (roadmap P2-T1).
#
#   scripts/dev-engine.sh            # build engine + fetch model into .dev/
#
# Result:
#   .dev/llama-server                         the engine binary
#   .dev/models/<file>.gguf                   the dev model
#
# The Windows host gets a CUDA build instead (P7-T3); this script is for dev
# and CI only. Release downloads from github.com are not reachable in cloud
# sessions, but `git clone` is, so the engine is built from source.
set -euo pipefail

# Pinned engine version. Bump only after the procedure in roadmap P7-T8.
LLAMA_CPP_TAG="b11582"

# Smallest instruct model that still chats coherently enough to eyeball.
DEV_MODEL_URL="https://huggingface.co/bartowski/SmolLM2-135M-Instruct-GGUF/resolve/main/SmolLM2-135M-Instruct-Q8_0.gguf"
DEV_MODEL_FILE="SmolLM2-135M-Instruct-Q8_0.gguf"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEV="$ROOT/.dev"
mkdir -p "$DEV/models"

if [ ! -x "$DEV/llama-server" ]; then
  SRC="$DEV/llama.cpp"
  if [ ! -d "$SRC" ]; then
    git clone --depth 1 --branch "$LLAMA_CPP_TAG" https://github.com/ggml-org/llama.cpp "$SRC"
  fi
  cmake -S "$SRC" -B "$SRC/build" -DCMAKE_BUILD_TYPE=Release \
    -DGGML_NATIVE=OFF -DLLAMA_CURL=OFF -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF \
    -DBUILD_SHARED_LIBS=OFF
  cmake --build "$SRC/build" --target llama-server -j"$(nproc)"
  cp "$SRC/build/bin/llama-server" "$DEV/llama-server"
fi

if [ ! -f "$DEV/models/$DEV_MODEL_FILE" ]; then
  curl -fL --retry 4 -o "$DEV/models/$DEV_MODEL_FILE.part" "$DEV_MODEL_URL"
  mv "$DEV/models/$DEV_MODEL_FILE.part" "$DEV/models/$DEV_MODEL_FILE"
fi

"$DEV/llama-server" --version 2>&1 | head -2
ls -la "$DEV/models"
