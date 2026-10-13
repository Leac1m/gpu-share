# GPU Share — Roadmap and progress log

**Purpose.** This file is the shared working memory for everyone (human or agent) touching this
project. It answers three questions: what is planned, what is already done, and what someone
learned the hard way that you should know before you start.

| | |
|---|---|
| Companion spec | [gpu-share-project.md](gpu-share-project.md) — the source of truth for *what* and *why* |
| This file | The source of truth for *state* — what is done, in progress, blocked, and why |
| Last updated | 13 October 2026 |

---

## How to use this file (read before you start work)

**Task IDs are permanent.** `P2-T5` always means the same task. Refer to them in commits, PRs,
and notes. Never renumber; if a task dies, mark it `[dropped]` and say why in a note.

**Status markers** go in the checkbox and an optional tag after the ID:

| Marker | Meaning |
|---|---|
| `- [ ] P3-T1` | Not started |
| `- [ ] P3-T1 [wip]` | Someone is on it right now |
| `- [ ] P3-T1 [blocked]` | Can't proceed — a note must say what is blocking it |
| `- [x] P3-T1` | Done, and the "done when" condition is actually met |
| `- [ ] P3-T1 [dropped]` | No longer planned — a note must say why |

**When you finish a task:** tick the box, update the phase counter in the dashboard, and update
"Last updated" above.

**When you learn something another agent would need:** add a note in that phase's Notes section.
Worth a note: a decision and its reasoning, a version that had to be pinned, a library that didn't
work, a measurement, a dead end, a gotcha that cost you an hour, an assumption you made that is
not in the spec. Not worth a note: routine progress, restating the task, or anything the code and
git history already say.

**Note format** (append at the end of the phase's Notes section, newest last):

```
> `P2-T5` · 2026-09-12 · <who you are>
> What you learned, in one or two sentences. Be concrete — name the version,
> the error, the number, the file.
```

**Never edit or delete someone else's note.** If a note is now wrong, add a new one that says so
and references the date of the one it supersedes.

**Decisions** that answer an open question go in [Decisions](#decisions), not only in a note.

---

## What to automate vs. verify by hand

No CI runner will ever have an RTX A2000, a Windows idle timer, or a real home router. That split
is permanent, so the strategy is: **push logic into pure functions so the untestable surface stays
small**, write automated tests around that surface, and verify the OS/hardware shims by hand once,
on real machines.

**Classify every task before starting it:**

| Kind | Example tasks | Do this |
|---|---|---|
| Pure logic | Policy loop precedence, VRAM estimator, request clamping, GGUF header parsing, protocol (de)serialization | Write the test first. This is cheap and it's where regressions are invisible-by-clicking (e.g. two pause reasons swap precedence). |
| Exposed surface / contract with the owner | Gatekeeper route allowlist, pairing auth matrix, "prompt text never stored" | Write the test **before** the feature. This is the executable form of the spec §7 safety claims — an agent adding a route later should hit a failing test, not just code review. |
| OS / hardware shim | `GetLastInputInfo`, NVML, GPU perf counters, real multicast, tray icons, installers | No CI test. Wrap behind a small trait/struct (e.g. a `WorldSnapshot`) so the *consumer* of the shim is still pure and testable; verify the shim itself manually on real hardware and log it in the phase's Notes section. |

**Concrete automated suites worth having, roughly in the order they become buildable:**

1. Protocol round-trip tests with checked-in golden JSON fixtures (catches Rust↔frontend↔other-install drift) — *Phase 0*
2. Policy loop precedence + hysteresis + away-countdown, table-driven (~40 cases) — *Phase 0, ported from the prototype's `policy()`*
3. Gatekeeper route allowlist: assert the exact 5 endpoints in spec §6.8, 404 on ~30 plausible others — *Phase 2, write before the gatekeeper*
4. Pairing auth matrix: unpaired / paired / unknown-cert × endpoint (15 cases) — *Phase 2/4*
5. "No prompt or reply text stored": run a chat with a magic string, grep the raw SQLite file for it — *Phase 2*
6. Request clamping: property test, `max_tokens` in the forwarded request never exceeds the owner's limit for any input — *Phase 2*
7. VRAM estimator, pinned against the real Phase 5 hardware measurements once they exist — *Phase 0/6, updated Phase 5*
8. GGUF header parser fuzz target: malformed input errors, never panics — *Phase 6*
9. Supervisor lifecycle against a fake `llama-server` binary (crash → restart, stop → process actually dead) — *Phase 2*
10. Download resume + SHA-256 against a local server that drops the connection mid-transfer — *Phase 6*

**Manual verification checklist** (real hardware/network only, tracked here rather than in CI):

- [ ] Windows idle timer and away-countdown feel right to the owner (`P5-T2`)
- [ ] Named-app pause list actually catches the owner's creative apps (`P5-T3`)
- [ ] GPU performance-counter pause fires when another app grabs the GPU, and not for a browser playing video (`P5-T4`, threshold in `world.rs`)
- [ ] NVML temperature hysteresis on the real A2000 — no flapping (`P5-T5`)
- [ ] Battery / plugged-in detection (`P5-T6`)
- [ ] The PC doesn't sleep while a friend's question is being answered, and does sleep again once sharing is idle (`P5-T9`)
- [ ] Real multicast discovery across the actual home router, including the client-isolation case (`P3-T2`, `P3-T7`). If multicast fails there, check whether `GPU_SHARE_IFACE` with the Wi-Fi adapter's name fixes it
- [ ] Windows network-profile (Private vs Public) first-run check (`P7-T2`)
- [ ] Installer + firewall rule on a clean Windows machine (`P7-T1`)
- [ ] Tray icon on both platforms (`P7-T5`)
- [ ] Tokens/sec, load time, VRAM, temperature on the real A2000 (`P5-T11`)
- [ ] Engine Job Object on Windows: closing the app kills `llama-server`, priority shows Below normal in Task Manager (`P2-T4`)

---

## Dashboard

| Phase | Goal | Done | Status |
|---|---|---|---|
| [0](#phase-0--foundations) | Foundations and decisions | 6 / 6 | Done |
| [1](#phase-1--clickable-prototype) | Clickable prototype | 5 / 10 | Build 0.2 in the repo and the Svelte app; feedback not yet gathered |
| [2](#phase-2--real-chat-end-to-end) | Real chat end to end | 10 / 10 | Done |
| [3](#phase-3--discovery) | Discovery on the LAN | 8 / 8 | Built and tested on one machine; the real home network check is still to do |
| [4](#phase-4--pairing-and-security) | Pairing and security | 7 / 7 | Done |
| [5](#phase-5--limits-wired-up) | Limits wired up | 10 / 11 | Built; the rules and measurements on the real laptop are still to do (`P5-T11`, manual checklist) |
| [6](#phase-6--model-library) | Model library | 7 / 8 | Download, verify and "fits your GPU" work end to end; publishing the catalog (D3) is open |
| [7](#phase-7--installers-updates-pilot) | Installers, updates, pilot | 0 / 9 | Not started |
| [L](#later-backlog) | Later (backlog) | 0 / 5 | Deferred by design |

---

## Where the project stands today

The repository holds the spec, the HTML prototype (build 0.2), the Svelte app over a mock for the
browser preview, and the real host and client in Rust behind the same UI. See the dashboard for
which phases are real.

| What exists | Where | State |
|---|---|---|
| Full project spec | [gpu-share-project.md](gpu-share-project.md) | v0.3, draft but detailed enough to build from |
| Clickable Phase 1 prototype | [gpu-share-preview.html](gpu-share-preview.html) | **Build 0.1** — single self-contained HTML file, 1246 lines, vanilla JS. The spec describes build 0.2 (queue shared by several friends); that file is not in this repo yet, see the `P1-T9` note |
| Shared protocol crate | `crates/protocol` | Announcement type, the 5-endpoint gatekeeper allowlist as data, catalog entry and VRAM estimate. 8 tests |
| Desktop app scaffold | `apps/desktop` | Tauri 2 + Svelte 5 port of prototype build 0.1 over a typed mock backend (`src/lib/mock/backend.svelte.ts`) |

What the prototype actually simulates today: both machines side by side; the share switch; a
policy loop (`policy()` / `tick()`) covering battery, schedule, named apps, GPU-heavy apps, idle
timer with resume countdown, and temperature hysteresis; model library with fake downloads and a
VRAM-fit check; pairing with a 6-digit code; a chat that streams at RTX A2000-ish speed; usage
counters per paired device; tray menu; toasts; and a test bench that fakes the real-world signals.

What it deliberately does not do: any networking, any real model, any persistence.

---

## Phase 0 — Foundations

Turning the prototype into a real codebase. None of this is described as a phase in the spec, but
Phase 2 can't start without it.

- [x] **P0-T1** — Decide the frontend framework (Svelte or React). See [Decisions](#decisions). *(spec §5)*
- [x] **P0-T2** — Decide the project name. "GPU Share" is a working title. *(spec §9)*
- [x] **P0-T3** — Scaffold the Tauri 2 workspace: Rust backend, web frontend, and a shared
      `protocol` crate for every message and API type used by both modes. Builds on Ubuntu.
      *(spec §5, §6.10)*
- [x] **P0-T4** — Port the prototype's screens into the chosen framework, with all data coming
      from a mock layer behind an interface that real calls can later replace without redesign.
      *(spec §8 Phase 1)*
- [x] **P0-T5** — Choose and document the multicast group, multicast port, and gatekeeper TCP
      port. Must not collide with LocalSend (224.0.0.167:53317). *(spec §6.6)*
- [x] **P0-T6** — CI that builds and tests on both Ubuntu and Windows.

**Done when:** `cargo build` and the frontend build both pass on Ubuntu and Windows, and the
ported UI behaves like the HTML prototype.

### Notes — Phase 0

> `P0-T3` · 12 September 2026 · Claude (Sonnet 5)
> What exists so far: root `Cargo.toml` workspace; `crates/protocol` with `discovery` (the
> `Announcement` type, golden fixture at `crates/protocol/tests/fixtures/announcement.json`),
> `gatekeeper` (the exact 5-endpoint allowlist from spec §6.8 as data, plus a test that fails if it
> drifts), and `models` (`CatalogEntry` and the VRAM estimate formula) — 8 tests, all passing,
> `cargo clippy -D warnings` clean. `apps/desktop` is a Svelte + TypeScript + Vite scaffold (via
> `npm create vite@latest -- --template svelte-ts`), builds clean, but it is **not yet a Tauri app**
> and still has the default Vite template content — P1's screens have not been ported in (that's
> `P0-T4`).
>
> **What's blocked, and the handoff:** turning `apps/desktop` into an actual Tauri window needs
> `cargo tauri init` plus Linux system libraries (`libwebkit2gtk-4.1-dev`,
> `libayatana-appindicator3-dev`, `librsvg2-dev`, `patchelf`, `build-essential`) that need `sudo`,
> which this environment doesn't have non-interactively. Whoever has a terminal with sudo on this
> machine should run:
> ```
> sudo apt update && sudo apt install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
>   librsvg2-dev patchelf build-essential curl wget file
> source "$HOME/.cargo/env"
> cargo install tauri-cli --version "^2"
> cd apps/desktop && cargo tauri init
> ```
> Point `cargo tauri init` at `apps/desktop` as the frontend dist and dev-server command it already
> has (`npm run build` / `npm run dev`), and add `crates/protocol` as a path dependency of the
> generated `src-tauri` crate. Once `src-tauri` exists, extend `.github/workflows/ci.yml` with a
> `tauri-build` job that `apt-get install`s the same packages before `cargo check` — GitHub-hosted
> Ubuntu runners have passwordless sudo, so that part isn't blocked, only local verification is.

> `P0-T3` · 12 September 2026 · Claude (Sonnet 5)
> Sudo step above is done and `cargo tauri init` has run (`apps/desktop/src-tauri` exists, committed
> in `35686a5`). Finished the wiring the handoff called for: added `apps/desktop/src-tauri` to the
> root workspace `members`, and added `protocol = { path = "../../../crates/protocol" }` to
> `apps/desktop/src-tauri/Cargo.toml` — neither had been done yet, so `cargo check` from
> `src-tauri` failed with "current package believes it's in a workspace when it's not" until fixed.
> `cargo check/test/clippy --workspace --all-targets` all pass now, protocol's 8 tests still green.
> Also added the missing `apt-get install` step (the same package list from the sudo command above)
> to the `rust-ubuntu` CI job, since the workflow would otherwise fail on GitHub's runner the same
> way it did locally before the packages were installed. Not yet pushed, so CI going green there is
> still unconfirmed — same caveat as the `P0-T6` note below. Marking `P0-T3` done since the scaffold
> itself (Rust backend + web frontend + shared protocol crate, building on Ubuntu) is complete;
> porting the Phase 1 screens into it is the separate `P0-T4`.

> `P0-T6` · 12 September 2026 · Claude (Sonnet 5)
> `.github/workflows/ci.yml` created: an Ubuntu job runs `cargo test --workspace` and
> `cargo clippy -- -D warnings`, a Windows job runs `cargo check --workspace` (compile-only —
> Windows CI has no GPU/NVML/desktop session, see "What to automate vs. verify by hand" above), and
> a third job builds the frontend. All three steps were run locally on Ubuntu and pass; the
> workflow itself hasn't executed on GitHub Actions yet since nothing has been pushed. Marked
> `[wip]` until a push/PR confirms it goes green there too — untick this note's confidence, not the
> task, if it doesn't.

> `P0-T5` · 12 September 2026 · Claude (Sonnet 5)
> Picked `239.192.152.143:53737` for multicast (organization-local scope, RFC 2365) instead of
> reusing anything in LocalSend's `224.0.0.0/24` global-scope block, and kept `50123` — already
> used as the spec's own placeholder — as the real gatekeeper HTTPS port. Nothing consumes these
> yet; they just need to land in `crates/protocol` (a `pub const` alongside `PROTOCOL_VERSION`)
> when Phase 3 starts.

> `P0-T4` · 12 September 2026 · Claude (Sonnet 5)
> Ported `docs/gpu-share-preview.html` into `apps/desktop/src` as Svelte 5 (runes) components
> over a typed mock backend: `src/lib/types.ts` (shared shapes), `src/lib/mock/catalog.ts` and
> `src/lib/mock/replies.ts` (ported verbatim from the prototype), and `src/lib/mock/backend.svelte.ts`
> — the one file that owns the fake policy loop, `tick()`/`policy()`, timers, pairing and streaming,
> exposed as two small interfaces (`HostBackend`, `ClientBackend`) via the exported singletons
> `hostBackend` / `clientBackend`. That module is the swap point for Phase 2+: a real implementation
> (Tauri commands on the host, the gatekeeper HTTP API on the client) should satisfy the same two
> interfaces so `src/lib/components/*.svelte` and `src/App.svelte` don't change. Screens live under
> `src/lib/components/` (one file per prototype screen, plus `Machine.svelte` for the shared
> titlebar/tabs/taskbar chrome); CSS was ported into `src/app.css` largely verbatim, keeping the
> `data-theme`/`prefers-color-scheme` split and breakpoints. `npm run build` and `npm run check`
> (svelte-check + tsc) both pass clean, and `cargo check --workspace --all-targets` is unaffected.
> Verified the golden path with a headless Chrome + puppeteer-core script (share toggle → pairing
> with matching code → streamed reply → test-bench Blender triggers the paused banner on both
> machines). Found and fixed one real bug while doing this: pushing a plain object into a Svelte 5
> `$state` array proxy-wraps it at push time, so mutating a held reference to the *original* object
> afterward (as the prototype's closure-based `msg.text += …` pattern would) never re-renders —
> the streaming loop now re-reads `client.msgs[msgIndex]` on every tick instead. Deliberately
> simplified: the client's taskbar clock reads the host's simulated `world.hour` directly (via
> `hostBackend.world`) rather than through `ClientBackend`, since the prototype itself treats both
> windows as one simulated environment with a single shared clock — worth revisiting when Phase 3
> gives each machine its own real clock.

> `P0-T6` · 2026-10-13 · Claude (Opus 5.5)
> Confirmed: CI run 3 on `main` (merge of PR #1, 2026-10-13) passed all three jobs, Ubuntu tests +
> clippy, Windows `cargo check`, frontend build. The workflow now also runs on pushes to every
> branch (work is pushed straight to a branch without PRs in some sessions, which otherwise got
> no CI at all) and runs `npm run check` and `npm test` in the frontend job.

---

## Phase 1 — Clickable prototype

Goal: friends see and click every screen before anything technical exists, and shape it.

- [x] **P1-T1** — Host screens: first-run welcome, tray menu, Share screen with switch and live
      status, resource limits, model library with downloads and removal, paired devices, pairing
      popup, and "paused because…" notices.
- [x] **P1-T2** — Client screens: device list with live statuses, pairing flow, streaming chat,
      model switching with a loading message, and the host-paused message.
- [x] **P1-T3** — Test bench that fakes real life on the friend's laptop (at desk / away, running
      apps, battery, heat, time of day) so every pause rule can be demonstrated.
- [ ] **P1-T4** — Publish the preview as a web link that opens in any browser, with every screen
      labelled as a preview.
- [ ] **P1-T5** — Run the feedback session with the host friend. Watch where he hesitates rather
      than asking whether he likes it. Cover: are the setting names clear, are the defaults right,
      which apps should pause sharing, does he want notifications, and can he explain in his own
      words what the switch does and what the app cannot do. *(spec §8 Phase 1)*
- [ ] **P1-T6** — Write the feedback up in this repo, including the exact wording he used for
      things he misread.
- [ ] **P1-T7** — Apply the agreed UI changes to the prototype.
- [ ] **P1-T8** — Confirm the settings defaults and the pre-filled pause-app list from what he
      said. Current placeholders: budget 6.5 GB, context 4096, away 5 min, pause apps Blender /
      DaVinci Resolve / Steam, temp 83→75 °C, plugged-in only, max reply 1024. *(spec §6.4)*
- [x] **P1-T9** — Bring `docs/gpu-share-preview.html` up to build 0.2 (the build spec v0.3
      describes) by copying it in from the project's Drive folder.
- [x] **P1-T10** — Bring the Svelte app's mock layer and screens up to spec v0.3: a queue shared by
      several friends with "You're 2nd in line" and Cancel on the client, test-bench friends who can
      ask questions, general pause reasons for friends vs. exact ones for the owner, client status
      labels Ready / In use / Paused, and limits shown in tokens with approximate words.
      *(spec §6.3, §6.4, §6.9)*

**Done when:** friends have tried it and the UI changes are agreed.

### Notes — Phase 1

> `P1-T1` `P1-T2` `P1-T3` · 2026-09-12 · project author
> Built as one self-contained HTML file rather than a framework app, so it can be handed to a
> non-technical friend as a link with nothing to install. The state shape in `fresh()` and the
> JSON formats follow spec §6 deliberately — reuse them when porting (P0-T4) so the mock/real
> swap stays a drop-in.

> `P1-T8` · 2026-09-12 · project author
> The defaults in the prototype are proposals, not decisions. Do not treat them as agreed until
> P1-T5 has happened. Note the budget is 6.5 GB in the prototype and 6 GB in the spec table —
> reconcile the two when the real setting is built.

> `P1-T9` · 2026-10-13 · Claude (Opus 5.5)
> Spec v0.3 was brought in from the Drive copy today, but the matching prototype (build 0.2,
> `gpu-share-preview.html`, dated 9 October 2026) was not: the Drive file is private and this
> session had no way to save it to disk. The file in `docs/` is still build 0.1, and the Svelte
> port (`P0-T4`) was made from build 0.1, so neither has the multi-friend queue. `P1-T10` should
> be done from build 0.2 once `P1-T9` lands, not from the spec text alone.

> `P1-T9` `P1-T10` · 2026-10-13 · Claude (Opus 5.5)
> Build 0.2 was copied in from the project author's Drive (`gpu-share-preview.html`, 81,669
> bytes, dated 9 October 2026), which supersedes the `P1-T9` note above. The Svelte port now
> follows it: `src/lib/mock/backend.svelte.ts` is a port of 0.2's queue, bench friends (Tunde,
> Ada) and general-vs-exact pause reasons. The UI boundary was reshaped while doing this so the
> real backend can drop in: screens read two view models, `HostView` and `ClientView`
> (`src/lib/types.ts`), and call `HostBackend` / `ClientBackend`; tab and tray state moved to
> `src/lib/ui.svelte.ts`. `ClientView.hosts` is a list (the real client sees any number of hosts),
> and `ClientView.turn` carries queued-with-position / loading / streaming. Preview-only chrome
> (`Preview.svelte`, `HostMachine`, `ClientMachine`, `Link`, `TestBench`) is the only code that
> touches the mock's `sim` object. Replies render through `src/lib/markdown.ts`, which escapes
> everything first (replies come from another machine and land in a webview that can call Tauri
> commands); `npm test` (vitest) covers it. Checked in headless Chromium: pairing codes match,
> "You're 2nd in line" + Cancel while Tunde is answered, Blender pauses both sides with the
> general reason on the client and the exact one on the host, no console errors.

---

## Phase 2 — Real chat end to end

Build the gatekeeper and engine supervisor and connect the chat screen to them. Developed on the
Ubuntu machine with a CPU build of llama-server and a tiny model — **no GPU needed for this phase**.

- [x] **P2-T1** — Vendor or locate `llama-server`; a CPU build plus a small GGUF model for dev.
      Pin the version. *(spec §5)*
- [x] **P2-T2** — Engine supervisor: one process per model, start/stop on demand, health check,
      restart on crash, failures surfaced in the UI. *(spec §6.2)*
- [x] **P2-T3** — Engine hardening: bind `127.0.0.1` only, random API key generated at each start,
      `--parallel 1`, `--slot-save-path` left disabled. *(spec §6.2)*
- [x] **P2-T4** — Windows Job Object wrapper: kill-on-close, memory cap, below-normal CPU
      priority. Stub or no-op on Linux. *(spec §6.2)*
- [x] **P2-T5** — Gatekeeper: HTTPS server (axum + rustls) exposing exactly the five endpoints in
      spec §6.8. Everything else returns 404.
- [x] **P2-T6** — Request queue: one request at a time, waiting clients told their position,
      engine work cancelled when the client disconnects or presses Stop. *(spec §6.8)*
- [x] **P2-T7** — Request clamping: cap `max_tokens`, ignore any parameter that could push
      resource use past the owner's limits. *(spec §6.8)*
- [x] **P2-T8** — Client chat wired to the gatekeeper: streaming replies, Stop button, model
      picker, markdown and code formatting, SQLite history on the client. *(spec §6.9)*
- [x] **P2-T9** — Per-device usage counters in SQLite (requests, tokens in/out, GPU seconds), with
      a test asserting prompt and reply text is never written anywhere. *(spec §6.8)*
- [x] **P2-T10** — Queue rules added in spec v0.3: at most one waiting request per paired device;
      when the next request wants a different model, stop the engine and start one for that model;
      when a pause triggers, turn away everything still waiting with the pause message.
      *(spec §6.3, §6.8)*

**Done when:** chat streams through the real gatekeeper and the real engine.

### Notes — Phase 2

> `P2-T2` · 2026-09-12 · project author
> One process per model, not llama-server's router mode: a known issue leaves each idle child
> holding ~600 MB of VRAM after its model is put to sleep, which is unaffordable on an 8 GB card.
> See llama.cpp issue #19379. Switching model = stop one process, start another; when sharing
> stops, VRAM use must return to zero. Do not "optimise" this back to router mode without
> re-checking that issue.

> `P2-T1` · 2026-10-13 · Claude (Opus 5.5)
> For cloud sessions (see `CLAUDE.md`): under the default "Trusted" network level the VM can reach
> GitHub release downloads, crates.io and npm, but `huggingface.co` is **not** on the default
> allowlist per the cloud-environments docs. So a prebuilt CPU `llama-server` from llama.cpp's
> GitHub releases should download fine, while the dev GGUF model will not unless the project
> author adds `huggingface.co` to the environment's allowed domains, or the model is fetched on a
> local machine. Not yet tried in a cloud session — confirm and correct this note when you do.

> `P2-T1` · 2026-10-13 · Claude (Opus 5.5)
> Supersedes the 2026-10-13 `P2-T1` note above on one point: in this cloud environment GitHub
> *release downloads* are refused by the proxy (HTTP 403 on github.com and api.github.com), but
> `git clone` works. So `scripts/dev-engine.sh` builds a CPU `llama-server` from source at the
> pinned tag **b11582** (about 4 minutes on 4 cores) into `.dev/`, and fetches SmolLM2-135M-Instruct
> Q8_0 (145 MB) from Hugging Face, which the project author has since allowed. The engine flags
> the supervisor relies on (`--alias`, `--no-slots`, `--offline`, `--api-key`, `--parallel`) were
> checked against that build's `--help`.

> `P2-T2`…`P2-T10` · 2026-10-13 · Claude (Opus 5.5)
> Where things are: `crates/netcore` (identity + pinned rustls), `crates/host` (`line.rs` the
> pure queue, `clamp.rs`, `store.rs`, `engine.rs` the supervisor, `gatekeeper.rs`, `core.rs`
> `HostCore`, `runtime.rs` the loop that opens/closes the port), `crates/client` (`api.rs`
> `HostClient`, `history.rs`, `runtime.rs` `ClientRuntime`). The Tauri app (`src-tauri/src`) only
> wires these up and pushes `host-view` / `client-view` events; `src/lib/real/backend.svelte.ts`
> is the TS side. Decisions worth knowing:
> * TLS is rustls with the **ring** provider everywhere (and `reqwest` 0.12), to keep aws-lc-rs and
>   its Windows build tools out. The gatekeeper serves HTTP/1.1 over its own accept loop so each
>   connection's client-certificate fingerprint can be attached to the request.
> * Queue position and load progress travel inside the chat stream as SSE events named
>   `gpushare` (`protocol::api::QueueEvent`, golden fixture `queue_events.json`); OpenAI clients
>   ignore named events. The forwarded engine body is rebuilt from an allowlist
>   (`clamp::FORWARDED_KEYS`), `model` is rewritten to the catalog id and `timings` /
>   `system_fingerprint` are dropped, because llama-server otherwise reports the model's file
>   path (which contains the owner's user name).
> * A pause cuts the running reply after `PAUSE_GRACE` (3 s), then the engine is stopped.
> * `Announcement` gained optional `reason` and `gpu` fields (older installs ignore them).
> * Tests: suite 3 (`tests/gatekeeper_surface.rs`, 35 paths that must 404), suite 5
>   (`tests/no_prompt_text_stored.rs`), suite 6 (proptest in `clamp.rs`), suite 9
>   (`tests/supervisor.rs`, against `src/bin/fake_llama_server.rs`). `tests/real_engine.rs` runs
>   the real engine and is `#[ignore]`d in CI: `scripts/dev-engine.sh && cargo test -p host --test
>   real_engine -- --ignored`. It passed here (SmolLM2 answered through the gatekeeper).
> * `P2-T4` is compiled (checked with `cargo check --target x86_64-pc-windows-gnu` and in CI) but not
>   yet run on Windows; see the manual checklist.
> * `P2-T8` is verified through `ClientRuntime` against a live gatekeeper (`tests/client_runtime.rs`)
>   and the app was launched under Xvfb, but clicking through chat in the real window needs pairing
>   (Phase 4).

---

## Phase 3 — Discovery

LocalSend-style announcement on the LAN, with fallbacks for routers that misbehave.

- [x] **P3-T1** — Announcement message type in the shared `protocol` crate, carrying a protocol
      version so mismatched installs can tell. *(spec §6.6, §6.10)*
- [x] **P3-T2** — UDP multicast announce and listen: on startup, on every status change, and a
      ~15 s heartbeat. Clients drop a device after three missed heartbeats.
- [x] **P3-T3** — Reply path: `POST /api/register` over HTTPS, falling back to a multicast reply
      with `"announce": false` if HTTPS fails.
- [x] **P3-T4** — Subnet scan fallback: `GET /api/info` across the local /24.
- [x] **P3-T5** — "Add by IP address" manual option.
- [x] **P3-T6** — On Ubuntu, pick the real LAN interface and ignore virtual ones (Docker bridges,
      VPNs). If ufw is on, tell the user which port to allow.
- [x] **P3-T7** — Detect client isolation / blocked multicast (guest Wi-Fi, some mesh routers) and
      explain it in plain language rather than failing silently.
- [x] **P3-T8** — Verify that with the switch off the host announces nothing and is invisible.

**Done when:** devices appear and disappear correctly on the real home network.

### Notes — Phase 3

> `P3-T1`…`P3-T8` · 2026-10-13 · Claude (Opus 5.5)
> How discovery works, in case the real network disagrees (the "done when" needs the actual home
> router, which no cloud session has):
> * Hosts multicast to `239.192.152.143:53737` (D7) from `crates/host/src/announce.rs` only while
>   sharing is on: at once on any status change, then every 15 s. Clients say hello once a minute
>   (`announce: true`) and hosts answer it straight back by unicast UDP with `announce: false`;
>   clients answer a newly heard host with `POST /api/register` and, if that fails, say the host can
>   be seen but not reached (usually a Windows network set to Public).
> * Clients also poll every listed host and every paired host's last address over HTTPS every 5 s.
>   A host whose port stops answering twice in a row disappears (about 10 s after its owner switches
>   off); one that goes quiet otherwise disappears after three missed heartbeats (45 s).
> * Subnet scan (`crates/client/src/discovery.rs`): 8 s after start if nothing has turned up, or on
>   "Look again": TCP connect to port 50123 on each address of the local /24, 64 at a time, 800 ms
>   timeout. Addresses that accept are checked with `/api/info` over TLS. Addresses that answer at
>   all (even with a refusal) count as "devices that can see us", except `.1` and `.254` (usually
>   the router). Zero is the client-isolation hint.
> * Interface choice (`netcore::lan::pick_interfaces`) keeps private IPv4 addresses and drops
>   anything named like a container bridge, VPN or VM network. `GPU_SHARE_IFACE=eth0` (comma-
>   separated names) overrides it; the cloud VM needed that because its only address is
>   192.0.2.2, which isn't a private range.
> * Installs with a different protocol *major* version still show up, as paused with "Runs a
>   different version of GPU Share".
> * Tested: multicast between the announcer and the client listener on this VM's eth0, off-host
>   invisible (`tests/discovery_lan.rs`), the /24 scan over 127.0.0.0/24, Add by IP, and
>   disappearing after a switch-off (`tests/client_runtime.rs`).

---

## Phase 4 — Pairing and security

Being on the same Wi-Fi must not be enough to use someone's GPU.

- [x] **P4-T1** — Per-install key pair and self-signed certificate (rcgen). The certificate's
      SHA-256 fingerprint is the device's permanent ID. *(spec §6.7)*
- [x] **P4-T2** — Certificate pinning in both directions after pairing.
- [x] **P4-T3** — `POST /api/pair` plus the owner-facing popup with a matching 6-digit code on
      both screens. Nothing is granted until the owner clicks Allow.
- [x] **P4-T4** — Device tokens issued on approval and stored in SQLite on the host.
- [x] **P4-T5** — Paired devices list with a Remove button per device.
- [x] **P4-T6** — Verify unpaired devices can read name, status and protocol version and nothing
      else: `/v1/models` and `/v1/chat/completions` must reject them.
- [x] **P4-T7** — Security review of the gatekeeper's exposed surface against the claims in spec
      §7. Every "cannot" in that table needs a test or a structural argument.

**Done when:** only paired devices can chat.

### Notes — Phase 4

> `P4-T1`…`P4-T7` · 2026-10-13 · Claude (Opus 5.5)
> * The 6-digit code is **derived**, not chosen: `protocol::pairing::pairing_code(host fingerprint,
>   client fingerprint, client nonce)`, each side using the fingerprints it actually sees on the
>   TLS connection. A TLS-terminating proxy makes the two screens differ; `tests/pairing.rs`
>   proves it with a real proxy. The golden value is pinned in a unit test: changing the formula
>   makes old and new installs show different codes.
> * `POST /api/pair` holds the request open until the owner answers (120 s limit), then returns
>   the device token. One popup at a time; a device turned away waits 60 s. The popup clears if
>   the friend gives up.
> * When the owner removes a device, the friend's next poll gets 401 and drops the token but
>   keeps its own chat history (`History::unpair_host`). "Forget" on the friend's side deletes both.
> * `docs/security-review.md` is the `P4-T7` review: each spec §7 "cannot" mapped to a test or a
>   structural argument, plus DoS mitigations and the residual risks accepted for the pilot.
>   Two fixes came out of it: engine start errors no longer carry the engine's last output line
>   (errors are logged to disk), and the gatekeeper now caps connections at 64 and times out
>   request headers after 10 s.
> * **End-to-end check of the real app**: `scripts/e2e/run.sh eth0` starts two app instances
>   (owner and friend) under Xvfb, driven by WebDriver (`tauri-driver` + `WebKitWebDriver`). It
>   passed here: multicast discovery, matching codes, Allow, a real SmolLM2 reply in the chat,
>   "1 question answered" on the owner's screen with no question text, and the host vanishing
>   from the friend's list when switched off. It found one real bug on its first run: plain
>   (non-async) Tauri commands run on the main thread with no tokio runtime, so any command that
>   spawns work must be `async fn` (noted in `src-tauri/src/commands.rs`).

---

## Phase 5 — Limits wired up

The policy loop with real signals, on the friend's laptop. This is the first real GPU test.

- [x] **P5-T1** — Policy loop running every couple of seconds, driving the Off / Available / Busy
      / Paused state machine and what clients see. The prototype's `policy()` is the reference
      implementation of the precedence order. *(spec §6.3)*
- [x] **P5-T2** — "Pause while I'm using the PC": Windows idle timer (`GetLastInputInfo`) with the
      resume-after-away delay.
- [x] **P5-T3** — "Pause when these apps are open": running-process watcher, plus a picker that
      offers the apps currently running.
- [x] **P5-T4** — "Pause when another app uses the GPU": Windows GPU performance counters (the
      ones Task Manager uses), because NVML often can't report per-app VRAM on Windows.
- [x] **P5-T5** — NVML: temperature limit with hysteresis (pause 83 °C, resume 75 °C) and a free-
      VRAM check immediately before loading a model.
- [x] **P5-T6** — "Only when plugged in": `GetSystemPowerStatus`.
- [x] **P5-T7** — Allowed hours: weekly schedule checked by the policy loop.
- [x] **P5-T8** — GPU memory budget enforced before load, and max conversation length passed to
      the engine as `-c`.
- [x] **P5-T9** — Free the GPU after idle, and keep the PC awake with `SetThreadExecutionState`
      while serving — releasing it as soon as the host goes idle or pauses.
- [x] **P5-T10** — Graceful mid-reply pause: the reply finishes or is cut within a few seconds and
      the client shows the reason with a retry, never a raw error. *(spec §6.3, §6.9)*
- [ ] **P5-T11** — First real GPU measurements on the RTX A2000: tokens per second, model load
      time, VRAM use, temperatures. Record the numbers in the notes below.

**Done when:** every pause rule works on the friend's laptop.

### Notes — Phase 5

> `P5-T1` · 2026-09-12 · project author
> Low CPU priority does not stop the engine competing for the GPU, and Windows gives apps no
> control over GPU scheduling. The pause rules are the only thing actually protecting the owner's
> experience — treat any regression in them as a release blocker, not a polish item.

> `P5-T1`…`P5-T10` · 2026-10-13 · Claude (Opus 5.5)
> * The decision is `crates/host/src/policy.rs::decide`, pure, with the prototype's precedence:
>   off → battery → hours → listed app → other app on the GPU → at the desk → away countdown →
>   temperature. 33 table cases plus hysteresis tests. A signal the machine can't read (`None`)
>   never pauses. "At the desk" means under 30 s since the last input; between 30 s and the away
>   time it's the countdown. Consequence worth telling the owner: with "Pause while I'm using the
>   computer" on, sharing stays paused until they've been away that long *after* switching it on.
> * Friend-facing reasons are gender-neutral in the real app ("They're using their computer",
>   "Their laptop is on battery"); the preview keeps "he/his" because it is about Dara.
> * Signals: `crates/host/src/world.rs`. Windows: `GetLastInputInfo`, `sysinfo` processes owned by
>   the current user (friendly names for the D6 apps, system processes hidden),
>   `\GPU Engine(*)\Utilization Percentage` via PDH summed per pid, ignoring our own and the
>   engine's pids, 15 % threshold (a guess: tune it on the real laptop; browser video can
>   reach 5–10 %), `GetSystemPowerStatus`, NVML for temperature, card name and free VRAM.
>   Linux: processes, `/sys/class/power_supply`, NVML if present; no idle time.
>   Compiled for Windows here (`--target x86_64-pc-windows-gnu`); none of it has run on Windows.
> * `GPU_SHARE_WORLD_FILE=path.json` replaces the real signals with a JSON file read every tick,
>   for development and `scripts/e2e/run.sh`, which now opens "Blender" that way and checks the
>   owner sees "Blender is open" while the friend sees only the general reason.
> * Free VRAM is checked with NVML right before a model loads (the loaded model's share counts
>   as free since it's stopped first). Lowering the budget below the loaded model unloads it.
>   `SetThreadExecutionState` runs on its own thread (`awake.rs`) because Windows ties it to the
>   calling thread; it's on only while the gate is open and a model is loaded or answering.
> * A cut or refused question gets an "Ask again" button in the chat (`P5-T10`).
> * Allowed hours are one daily window (the UI's "Evenings and nights"), not a per-weekday
>   schedule; the prototype never had one. Add it if the owner asks.
> * Found by the e2e run, fixed, and covered by a test: `HostCore::view()` deadlocked once
>   signals were present, because a `MutexGuard` temporary inside a struct literal lives until the
>   end of the literal and the same mutex was locked again further down.

---

## Phase 6 — Model library

- [ ] **P6-T1** — Catalog hosted as a JSON file (GitHub Releases is the default option), with two
      or three starter models — one 3-4B, one 7-8B, both 4-bit — after checking their licences.
      *(spec §6.5, §9)*
- [x] **P6-T2** — Catalog fetch and cache in the app, with a sensible offline fallback.
- [x] **P6-T3** — Resumable downloads with a free-disk-space check first.
- [x] **P6-T4** — SHA-256 verification after download. Models saved only into the app's own
      models folder.
- [x] **P6-T5** — GGUF header parsing: layer count, attention heads, embedding size.
- [x] **P6-T6** — VRAM estimate (file size + KV cache + ~0.5 GB overhead) driving the "Fits your
      GPU" badge and the greyed-out state.
- [x] **P6-T7** — Per-model controls: enable/disable (disabled models hidden from clients), set as
      default, remove — unloading first if the model is loaded.
- [x] **P6-T8** — Advanced toggle: add from a Hugging Face link.

**Done when:** download, verify, and "fits your GPU" work end to end.

### Notes — Phase 6

> `P6-T1`…`P6-T8` · 2026-10-13 · Claude (Opus 5.5)
> * The starter catalog is `crates/host/catalog.json`, bundled into the app: **Qwen3 4B and 8B
>   at Q4_K_M** from the official `Qwen/Qwen3-*-GGUF` repos, URLs pinned to a commit, sizes and
>   SHA-256 taken from Hugging Face's API on 2026-10-13, licence Apache 2.0 per the model cards.
>   Thinking is turned off with `--chat-template-kwargs {"enable_thinking":false}` (D5). Plus the
>   135M dev model, shown only in debug builds or with `GPU_SHARE_DEV=1`. `P6-T1` stays open
>   because the file isn't *hosted* anywhere yet: the app fetches `GPU_SHARE_CATALOG_URL` when
>   set (cached for offline starts, falling back to the bundled copy), so once D3 is decided,
>   publishing the JSON and setting a default URL is all that's left.
> * Every catalog entry is validated before use (`library::validate`): the id becomes a file name
>   so it must be a plain slug, the SHA-256 must be 64 hex, downloads only from `huggingface.co`
>   over HTTPS.
> * Downloads (`library::download`) resume a `.part` file with an HTTP Range request, hash the
>   existing part first so the whole file is always checked, start over cleanly if the server
>   ignores the range, and need the size plus 1 GB free first. Suite 10 is `tests/library.rs`
>   (local server that drops the connection mid-transfer). A real 105 MB download from Hugging
>   Face through the same code passed here (`-- --ignored`).
> * GGUF header parser (`gguf.rs`) with hard caps on every length and a proptest that random or
>   corrupted bytes never panic (suite 8). It read the real SmolLM2 file correctly. Installed
>   models use the shape from their own file; catalog shapes are only for models not yet
>   downloaded.
> * "Add from a Hugging Face link" (`P6-T8`) takes the size and SHA-256 from the `X-Linked-Size`
>   / `X-Linked-Etag` headers on Hugging Face's redirect (so it must not follow redirects), so
>   those downloads are verified too. Models added this way are kept in `custom_models.json`.
> * Removing a model that is answering someone is refused with a message; a loaded but idle
>   model is unloaded first. "Make default" puts a model first in friends' pickers.
> * `scripts/e2e/run.sh` now removes the dev model on the owner's Models screen, adds it back
>   (a real 145 MB download, checksum verified), and has the friend chat with it.
> * The dev profile now builds dependencies without debug info: `target/` had reached 18 GB and
>   filled the cloud session's disk.

---

## Phase 7 — Installers, updates, pilot

- [ ] **P7-T1** — NSIS per-user installer for Windows, adding a firewall rule for private
      networks. No separate Tailscale, Python, or CUDA install steps. *(spec §3 R1, §6.10)*
- [ ] **P7-T2** — First-run check that the Windows network profile is Private, with a plain-
      language explanation of how to change it.
- [ ] **P7-T3** — CUDA runtime downloaded on first run rather than shipped inside the installer
      (several hundred MB).
- [ ] **P7-T4** — Ubuntu client packages: `.deb` for 22.04 and 24.04 (pulling in WebKitGTK 4.1)
      and an AppImage. No engine bundled.
- [ ] **P7-T5** — Tray icon working on both platforms: native on Windows, AppIndicator on Ubuntu.
- [ ] **P7-T6** — Tauri auto-updater with signed update packages, so engine security fixes reach
      hosts without them doing anything.
- [ ] **P7-T7** — Code-signing certificate: cost, timing, and purchase decision. Without one,
      Windows shows "Windows protected your PC" — acceptable for the pilot if the friend is
      warned, not acceptable for wider release. *(spec §9)*
- [ ] **P7-T8** — Pin the engine version per release and define the test procedure before bumping
      it. llama.cpp moves quickly.
- [ ] **P7-T9** — Two-week pilot with the friend, then review the usage counts and his experience.

**Done when:** the two-week pilot is completed with no blocking issues.

### Notes — Phase 7

_None yet._

---

## Later (backlog)

Deliberately out of scope for now. The design keeps each of these possible; none is built.

- [ ] **L-T1** — Remote access over an embedded Tailscale connection, so sharing works outside the
      home network. The network layer is kept separate for this. *(spec R11)*
- [ ] **L-T2** — Several hosts, with requests sent to whichever friend's PC is available.
- [ ] **L-T3** — Renting: pricing, payments, per-device billing from the usage counts, terms of
      use. Per-device identity and usage counts exist from day one for this. *(spec R10)*
- [ ] **L-T4** — CPU-only host mode for Linux.
- [ ] **L-T5** — Local OpenAI-compatible API on the client's `localhost` that forwards to the
      chosen host, so editors and scripts can use the host's GPU. *(spec §6.9 — pull forward into
      Phase 2 if it turns out to be cheap)*

### Notes — Later

_None yet._

---

## Decisions

Open questions from spec §9. When you resolve one, fill in the decision and the date, and tick the
task that depended on it. If you resolve something not listed here, add a row.

| # | Question | Status | Decision | Date |
|---|---|---|---|---|
| D1 | Project name | Decided | "GPU Share" — the working title is kept as the real name | 12 September 2026 |
| D2 | Frontend framework (Svelte or React) | Decided | Svelte — smaller runtime, less boilerplate for a state-machine-heavy UI (share switch, policy states, streaming chat) | 12 September 2026 |
| D3 | Where to host the model catalog and update files | Open | GitHub Releases is the default option. *Recommended 13 October 2026, not yet confirmed:* bundle the catalog in the app until Phase 6, then GitHub | — |
| D4 | Code-signing certificate: cost and timing | Open | *Recommended, not yet confirmed:* none for the pilot; warn friends about the SmartScreen prompt and decide before any wider release | — |
| D5 | Which models go in the first catalog (licences checked) | Open | *Recommended, not yet confirmed:* Qwen3 4B and Qwen3 8B at Q4_K_M (Apache 2.0), thinking mode off so it doesn't eat the 1,024-token reply cap. Licences (Apache 2.0) and SHA-256s checked at the source on 13 October 2026 and built into the app's catalog; awaiting the project author's confirmation | — |
| D6 | Which apps to pre-fill in the pause list | Open | Ask the friend during P1-T5. *Recommended starting list:* Blender, DaVinci Resolve, Premiere Pro, After Effects, Photoshop, Unreal Engine, Unity, OBS | — |
| D7 | Multicast group and ports (must not collide with LocalSend) | Decided | Multicast `239.192.152.143:53737` (organization-local scope per RFC 2365, distinct from LocalSend's global-scope `224.0.0.167:53317`). Gatekeeper HTTPS on `50123` (the spec's own placeholder, kept as the real default). | 12 September 2026 |
| D8 | How renting would work, if it happens | Deferred | Not part of the first build (project author). Keep per-device usage counts; build nothing for pricing, payments or billing | 13 October 2026 |
| D9 | Per-friend priority or daily limit (spec §9, added in v0.3) | Open | *Recommended, not yet confirmed:* neither in the first build; first come first served, one waiting request per device and the reply cap are enough until the pilot says otherwise | — |
| D10 | What the first build is for | Decided | A working prototype the project author can try with friends comes first (project author). *Proposed cut, not yet confirmed:* real chat with the queue, multicast discovery plus "Add by IP", pairing, the share switch with the idle and other-app-on-GPU pause rules, two fixed models, unsigned installers, no auto-updater | 13 October 2026 |

---

## Change log

| Date | Change |
|---|---|
| 12 September 2026 | Roadmap created from spec v0.2. Phase 1 prototype tasks marked done. |
| 13 October 2026 | Spec updated to v0.3. Added `P1-T9`, `P1-T10`, `P2-T10` for what v0.3 introduced. Decisions D8 and D10 recorded, D9 added, recommendations noted on D3–D6. "Where the project stands today" corrected to mention the scaffold. Cloud-session setup added (`CLAUDE.md`, `scripts/cloud-setup.sh`). |
