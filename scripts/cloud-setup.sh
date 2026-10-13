#!/bin/bash
# Installs the Linux system libraries Tauri needs, in Claude Code cloud
# sessions only. Run by the SessionStart hook in .claude/settings.json.
# Same package list as the rust-ubuntu job in .github/workflows/ci.yml —
# keep the two in step.

if [ "$CLAUDE_CODE_REMOTE" != "true" ]; then
  exit 0
fi

# Already installed (resumed session or cached environment): nothing to do.
if dpkg -s libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf >/dev/null 2>&1; then
  exit 0
fi

SUDO=""
if [ "$(id -u)" -ne 0 ]; then
  SUDO="sudo"
fi

export DEBIAN_FRONTEND=noninteractive
$SUDO apt-get update -qq
$SUDO apt-get install -y -qq libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
  librsvg2-dev patchelf build-essential curl wget file

# Never block the session on a failed install; cargo will say what is missing.
exit 0
