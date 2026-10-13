# GPU Share

A Tauri 2 desktop app that lets friends on one home network use a friend's idle NVIDIA GPU to run
AI chat models. Spec: `docs/gpu-share-project.md`. State of the work: `docs/roadmap.md`.

## Before starting any task

Read `docs/roadmap.md`. It holds the task IDs (`P2-T5` and so on), what is done, the notes earlier
sessions left, and the Decisions table. Follow its rules: tick tasks only when their "done when"
is met, add a dated note for anything the next session would need, never edit someone else's note,
and refer to task IDs in commits and PRs.

## Current priority

A working prototype the project author can try with friends on a real home network (roadmap D10).
Renting is out of the first build (D8): keep per-device usage counts, build nothing for pricing,
payments or billing. Items marked "recommended, not yet confirmed" in the Decisions table are
proposals, not decisions.

## Commands

```
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings

cd apps/desktop
npm ci
npm run check        # svelte-check + tsc
npm test             # vitest
npm run build

scripts/dev-engine.sh                                         # CPU llama-server + tiny model in .dev/
cargo test -p host --test real_engine -- --ignored            # chat through the real engine
cargo check --workspace --target x86_64-pc-windows-gnu        # Windows shims (needs mingw-w64)
```

CI (`.github/workflows/ci.yml`) runs the same on Ubuntu and a compile-only check on Windows.

## Layout

- `crates/protocol` — every message and API type shared by host and client. Change wire formats
  here first, with a golden JSON fixture.
- `crates/netcore` — per-install identity (certificate) and TLS pinned by fingerprint.
- `crates/host` — Share mode: queue, clamping, engine supervisor, gatekeeper, usage store.
- `crates/client` — Use mode: gatekeeper client, chat history, the client runtime.
- `apps/desktop/src` — Svelte 5 (runes) UI. Screens read `HostView` / `ClientView` and call
  `HostBackend` / `ClientBackend` (`src/lib/types.ts`). `src/lib/mock/` drives the browser preview,
  `src/lib/real/` the Tauri app; `src/lib/backend.ts` picks one.
- `apps/desktop/src-tauri` — wires the crates into Tauri commands and view events.
- `docs/gpu-share-preview.html` — the original clickable prototype the Svelte app was ported from.

## Rules that are easy to break

- The gatekeeper exposes exactly the five endpoints in spec §6.8. `crates/protocol` has a test that
  fails if the list drifts; do not add a route without changing the spec first.
- Prompt and reply text is never stored on the host.
- One `llama-server` process per model, never router mode (roadmap `P2-T2` note).
- Write the test first for pure logic and for anything in spec §7. OS and hardware shims go behind
  a small trait and are verified by hand; see "What to automate vs. verify by hand" in the roadmap.

## Cloud sessions

`scripts/cloud-setup.sh` installs the Tauri system libraries at session start (it does nothing
locally). There is no GPU, no Windows, and no LAN peer in a cloud VM, so Phase 5 hardware work, the
Windows shims and real multicast discovery cannot be verified there: build them behind traits,
test the pure logic, and leave the manual check unticked in the roadmap. `huggingface.co` is not
reachable under the default network level (roadmap `P2-T1` note).
