# GPU Share (working title)

**Use a friend's idle graphics card over the home network, safely and on their terms.**

| | |
|---|---|
| Document version | 0.4 (draft) |
| Last updated | 15 October 2026 |
| Status | Phase 1 prototype updated for several friends sharing one host. Nothing real behind it yet. |
| Pilot setup | One host (Windows, NVIDIA RTX A2000 8 GB) and several clients on the same local network: the developer's Ubuntu PC (no GPU) plus other friends. Ubuntu computers can host too, for Ubuntu-to-Ubuntu testing |

---

## For friends: the short version

You don't need to read past this section unless you're curious.

This app lets friends on your home network use your graphics card to run AI chat models, but only when you allow it. You install it like any other Windows program, and it lives as a small icon next to your clock. It has one main switch: **Share my GPU**. When the switch is off, the app does nothing, and nobody can see or reach your computer through it.

When the switch is on, your friends' computers can send questions to an AI model running on your graphics card and get answers back. If several friends ask at once, they take turns: your card answers one question at a time. That is the only thing it can do. The app has no way to open, read, copy, or change your files. It doesn't show your screen and doesn't let anyone control your computer.

You set the rules in a settings screen: which hours sharing is allowed, how much of your graphics card it may use, and when it should step aside on its own (for example when you open Blender or a game, when your laptop is on battery, or when the GPU gets hot). If you're using your computer, the app gets out of the way. Your friends see that you're busy, never which program you opened.

Before anything real is built, we'll make a clickable preview of every screen. You'll get to try it and tell us what's confusing or missing, so the finished app works the way you expect.

---

## 1. Background

The client's PC has no GPU, so running AI models locally on it is slow or impossible. Friends who do creative work own PCs with capable NVIDIA GPUs that sit idle much of the day.

Early research looked at pooling several GPUs into one big virtual GPU across the network (llama.cpp RPC, exo, GPUStack). That approach exists, but it is fragile, slow outside a fast local network, and awkward on Windows. This project takes the simpler route: each host PC runs a complete AI model on its own GPU, and clients send it chat requests over the network.

### Goals

- A Windows app that a non-technical person can install, switch on and off, and configure without help.
- Sharing never gets in the owner's way. When they need their PC, the app steps aside and frees the GPU.
- No path from the network to the owner's files.
- Clients on the same network find available hosts automatically, the way LocalSend finds devices, and chat with the models each host has chosen to offer.
- A client build for Ubuntu (and Windows).

### Not in scope for now

- Splitting one model across several PCs.
- Access over the internet (planned for later, see section 8).
- Renting GPUs for money. The design keeps this possible (per-device identity and usage counts) but nothing is built for it yet.

---

## 2. People and devices

| Role | Who | Device | Notes |
|---|---|---|---|
| Host | Friend (non-technical) | Windows, NVIDIA RTX A2000 8 GB, 16 GB RAM | Confirmed. Ampere, 8 GB GDDR6, roughly 288 GB/s memory bandwidth, low power draw. Comfortable with 7-8B models at 4-bit. |
| Client | Project developer | Ubuntu PC, no GPU | Also the developer and tester. |
| Clients | Other friends | Ubuntu or Windows PCs | Share the one host with the developer, taking turns through the queue. |
| Network | Everyone | Same home Wi-Fi/LAN | No internet relay needed for the pilot. |

What 8 GB of VRAM means in practice: a 7-8B model at 4-bit quantization takes roughly 5 GB, plus conversation memory and overhead, for about 5.5-6 GB in total. That runs well when the owner isn't using the GPU. There is no room to share the GPU while the owner is doing GPU-heavy work, so the app pauses completely in that case.

---

## 3. Requirements

| ID | Requirement | Notes |
|---|---|---|
| R1 | One installer for Windows, no technical steps | No separate Tailscale, Python, or CUDA install. |
| R2 | A single on/off switch for sharing | Off means the engine is stopped, VRAM is freed, and the PC is invisible on the network. |
| R3 | Configurable resource limits | Plain-language settings. See section 6.4. |
| R4 | File safety | Enforced by design, not by promises. See section 7. |
| R5 | No noticeable impact on the owner | Automatic pausing, idle unloading, low priority. |
| R6 | Automatic discovery on the LAN | LocalSend-style device list with live status. |
| R7 | Chat with the models each host offers | Streaming replies, model picker, stop button. |
| R7a | Several friends can share one host | One request at a time, first come first served, with each waiting client told its place in line. See section 6.8. |
| R8 | Client runs on Ubuntu | Also Windows. |
| R8a | Ubuntu computers can host too | With an NVIDIA card, or on the processor alone (slow; small models). Added in v0.4 for Ubuntu-to-Ubuntu testing. See section 6.11. |
| R9 | Hosts can add and remove models | Curated catalog with "fits your GPU" badges. |
| R10 | Ready for renting later | Per-device identity and usage counts from day one. |
| R11 | Ready for remote access later | Network layer kept separate so Tailscale can be added. |

---

## 4. How it works

```
HOST (Windows + NVIDIA GPU)                   CLIENT (Ubuntu or Windows)
---------------------------                   --------------------------
Tray icon + settings screen                   Device list
Policy loop (limits, pause rules)             Chat screen
Gatekeeper  <----- HTTPS, paired only -----   Local OpenAI-style API
   |  (localhost only)                        Chat history (SQLite)
Engine supervisor
   |
llama-server  ---->  GPU
Models folder (owned by the app)

Discovery: both sides announce themselves with UDP multicast on the LAN,
with a subnet scan as a fallback.
```

A typical session goes like this. The owner flips **Share my GPU** on. The host starts announcing itself on the network as "Ready". The client's device list shows it within a few seconds. The user picks the host, sees its models, and starts chatting. The host's gatekeeper checks that the client is paired, forwards the request to the engine, and streams the reply back. If the owner opens a GPU-heavy app, the policy loop pauses sharing, the engine stops, VRAM is freed, and the client shows "Paused: he's using his computer". If another friend's question is being answered when a request arrives, the request waits in line and the client shows its place.

---

## 5. Tech stack

| Layer | Choice | Why |
|---|---|---|
| App framework | **Tauri 2** (Rust backend, web frontend) | One codebase builds the Windows installer and Ubuntu packages. Supports tray icons on both. Small binaries. |
| Frontend | Web UI (framework to be decided, e.g. Svelte or React) | The same UI code runs in the browser for the Phase 1 prototype and inside the app later. |
| Backend language | Rust | Good libraries for networking, TLS, GPU monitoring, and Windows system APIs. |
| AI engine | llama.cpp `llama-server` (CUDA build on Windows) | Fast, OpenAI-compatible API, runs GGUF models, actively maintained. |
| Local storage | SQLite | Chat history on the client, usage counts and paired devices on the host. |
| Security | rustls, self-signed certificates generated per install | Encrypted connections and device identity without any central server. |
| GPU monitoring | NVML (NVIDIA's monitoring library) plus Windows performance counters | Temperature, free VRAM, and per-app GPU use. |

Alternative considered: **Flutter**, which LocalSend itself is built with. It is a reasonable choice if the team prefers Dart.

Useful Rust crates (to verify during Phase 2): `tokio`, `axum`, `reqwest`, `rustls`, `rcgen`, `nvml-wrapper`, `windows` (Win32 APIs), `sysinfo`, `rusqlite`.

---

## 6. Components

### 6.1 One app, two modes

Every install is the same app with two modes:

- **Share mode (host).** On Windows, only appears on machines with a supported NVIDIA GPU. On Ubuntu it is always available, with or without a GPU (section 6.11). Contains the switch, the settings, the model library, and the paired-devices list.
- **Use mode (client).** Available everywhere. Contains the device list and chat.

Neither installer contains the AI engine, which keeps them small: it is downloaded and checked on first use (section 6.10). The app opens in Share mode where sharing is the point (an NVIDIA card, or sharing already set up) and in Use mode otherwise.

### 6.2 AI engine and supervisor (host)

The app bundles `llama-server` and controls it through a supervisor inside the Rust backend.

- **One process per model.** `llama-server` has a "router mode" that can load and unload several models without restarting. However, a known issue in router mode leaves each idle child process holding roughly 600 MB of VRAM after its model is put to sleep. On an 8 GB card that matters. So the supervisor runs one `llama-server` process per model, and switching models means stopping one process and starting another. When sharing stops, VRAM use returns to zero.
- **Localhost only.** The engine listens on `127.0.0.1` with a random API key generated at each start. Nothing on the network can talk to it directly.
- **One request at a time** (`--parallel 1`). Other requests wait in the gatekeeper's queue.
- **Windows Job Object.** The engine runs inside a Job Object with kill-on-close (it can never outlive the app), a memory cap, and below-normal CPU priority.
- **Features left off.** Slot saving (`--slot-save-path`) stays disabled because it writes files to disk.
- **Health checks.** The supervisor restarts the engine if it crashes and reports failures to the UI.

### 6.3 Policy loop and states (host)

All limits feed one policy loop that runs every couple of seconds. It reads the system signals (switch, idle time, running apps, GPU use, temperature, power, schedule) and decides the host's state. That state controls both the engine and what clients see.

| State | Engine | What clients see |
|---|---|---|
| Off | Stopped | Nothing. The host doesn't announce itself. |
| Available | Stopped until a request arrives, then loads the requested model | "Ready" |
| Busy | Generating a reply | "In use" (new requests wait in line) |
| Paused (reason) | Stopped, VRAM freed | "Paused: he's using his computer", "Paused: his laptop is on battery", etc. |

When a pause triggers mid-reply, the current reply finishes or is cut off within a few seconds, and the client gets a clear message instead of an error. Requests still waiting in line are turned away with the same message, so nobody waits on a host that won't answer.

The owner and his friends see different pause reasons. The owner sees the exact cause ("Blender is open"). Friends see a general one ("He's using his computer"), so the app never tells them which programs he has open.

Setting the engine to low CPU priority doesn't stop it competing for the GPU, and Windows gives apps no control over GPU scheduling. The pause rules are what actually protect the owner's experience.

### 6.4 Settings: resource limits (host)

Defaults below are proposals, to be revisited after friends try the Phase 1 prototype.

| Setting (as the owner sees it) | Proposed default | How it's enforced |
|---|---|---|
| Share my GPU | Off | Stops announcing and stops the engine. |
| Resume sharing after restart | Off | If off, sharing starts switched off after every reboot. |
| GPU memory budget | 6 GB of 8 GB | Before loading, the app estimates the model's VRAM needs (section 6.5). Models over budget are greyed out. Free VRAM is checked through NVML right before loading. |
| Max conversation length | 4,096 tokens (about 3,000 words) | Passed to the engine as the context size (`-c`). The second-biggest VRAM factor after model size. |
| Pause while I'm using the PC | On, resume after 5 minutes away | Windows idle timer (`GetLastInputInfo`). |
| Pause when these apps are open | Pre-filled with his creative apps and games | Watches the running-process list. The owner can add apps from a list of what's currently running. |
| Pause when another app uses the GPU | On | Windows GPU performance counters (the same ones Task Manager uses), because NVML often can't report per-app VRAM on Windows. |
| Temperature limit | Pause at 83°C, resume at 75°C | NVML temperature. The gap between the two numbers stops it flicking on and off. |
| Only when plugged in | On | `GetSystemPowerStatus`. |
| Allowed hours | Any time | Weekly schedule checked by the policy loop. |
| Max reply length | 1,024 tokens (about 750 words) | The gatekeeper caps `max_tokens` on every request. This also limits how long other friends wait in line. |
| Free the GPU after idle | 5 minutes | The supervisor stops the engine after this long without requests. |

The settings screen shows limits in tokens, with an approximate word count next to each (1,000 tokens is about 750 words), so the owner sees the real unit and a number he can picture.

While serving, the app keeps the PC awake with `SetThreadExecutionState`, and releases that as soon as it goes idle or pauses.

### 6.5 Settings: model library (host)

The model list works like a small app store.

**Catalog.** A curated list hosted by the project (a JSON file on GitHub is enough). The owner picks from it. An "add from a Hugging Face link" option sits behind an Advanced toggle. Example entry:

```json
{
  "id": "example-8b-q4km",
  "name": "Example 8B (balanced)",
  "description": "Good general chat model. Fits 8 GB cards.",
  "file_url": "https://huggingface.co/<org>/<repo>/resolve/main/<file>.gguf",
  "size_bytes": 4920000000,
  "sha256": "<hash>",
  "quantization": "Q4_K_M",
  "license": "<license name and link>",
  "default_context": 4096
}
```

For the pilot, the catalog should start with two or three models: one small (3-4B) and one mid-size (7-8B), both at 4-bit.

**Downloads.** Resumable, with a free-disk-space check first and SHA-256 verification afterwards. Models are saved only into the app's own models folder.

**VRAM estimate.** The app reads the GGUF file header to get the layer count, attention heads, and embedding size, then estimates:

```
VRAM ≈ model file size
     + KV cache (2 × layers × context × kv_heads × head_dim × bytes per value)
     + ~0.5 GB overhead
```

The result drives the "Fits your GPU" badge and the greyed-out state.

**Per-model controls.** Enable or disable (disabled models are hidden from clients), set as default, and remove. Removing a model that is loaded unloads it first.

**Model file safety.** GGUF files are data, not programs. Parsing bugs have been found and fixed in llama.cpp before, so the app only downloads from the catalog or Hugging Face, verifies hashes, and keeps the engine updated.

### 6.6 Discovery (LocalSend-style)

LocalSend announces each device with a UDP multicast message to `224.0.0.167` on port `53317`. Other devices reply through the new device's HTTP API, and if multicast fails, LocalSend falls back to sending a request to every address on the local subnet. This project copies that pattern but uses its **own multicast address and port** (to be chosen) so it never collides with LocalSend on the same machine.

Announcement message (sent by hosts and clients):

```json
{
  "protocol": "1.0",
  "id": "<SHA-256 fingerprint of this device's certificate>",
  "name": "Friend's Laptop",
  "os": "windows",
  "app_version": "0.1.0",
  "modes": ["share", "use"],
  "status": "available",
  "port": 50123,
  "announce": true
}
```

The port number is a placeholder until the real ports are chosen. `status` is only sent by devices in Share mode.

Rules:

- Devices announce on startup, on every status change, and every ~15 seconds as a heartbeat.
- Clients remove a device from the list after three missed heartbeats.
- Other devices reply over HTTPS (`POST /api/register`), or by multicast with `"announce": false` if HTTPS fails.
- Fallbacks: a scan of the local /24 subnet (`GET /api/info` on each address) and a manual "Add by IP" option.
- When the owner's switch is off, the host stops announcing entirely and is invisible.

Network issues to handle:

- Windows must have the network set to **Private**. The installer adds a firewall rule for private networks (or Windows asks on first run).
- Guest Wi-Fi and some mesh routers block device-to-device traffic ("client isolation"). The app should detect this and explain it in plain language.
- On Ubuntu, the client should ignore virtual interfaces (Docker bridges, VPNs) and use the real LAN interface. Ubuntu's firewall (`ufw`) is off by default; if it's on, the app should say which port to allow.

### 6.7 Pairing (like Bluetooth)

Being on the same network doesn't make a device trusted, since anyone on the Wi-Fi could see the host. Pairing fixes that.

1. On first run, each install generates its own key pair and self-signed certificate. The certificate's fingerprint is the device's permanent ID.
2. The client asks to pair. The host shows a popup: "[Name] wants to use your GPU", with a 6-digit code. The client shows the same code.
3. The owner checks the codes match and clicks **Allow**.
4. The host stores the client's fingerprint and issues a device token. From then on, both sides only accept each other's known certificate.
5. The host's settings show a **Paired devices** list with a **Remove** button for each.

### 6.8 Gatekeeper API (host)

The gatekeeper is the only part of the host reachable from the network. It is an HTTPS server (`axum` + `rustls`) with exactly these endpoints:

| Endpoint | Who can call it | What it does |
|---|---|---|
| `GET /api/info` | Anyone on the LAN | Returns name, status, and protocol version. No model list. |
| `POST /api/register` | Anyone on the LAN | Discovery reply (section 6.6). |
| `POST /api/pair` | Anyone on the LAN | Starts pairing. Nothing happens until the owner clicks Allow. |
| `GET /v1/models` | Paired devices | Lists the models the owner has enabled. |
| `POST /v1/chat/completions` | Paired devices | OpenAI-compatible chat. Streams the reply from the engine. |

Everything else returns 404. The gatekeeper also:

- caps `max_tokens` and ignores request parameters that could raise resource use beyond the owner's limits;
- queues requests from all paired devices, first come first served, with one running at a time, and tells each waiting client its place in line;
- allows each paired device one request in the queue at a time, so one friend can't fill the line;
- when the next request in line wants a different model from the one loaded, stops the current engine process and starts one for the new model, which adds that model's load time (estimated at 4 to 9 seconds on the host, to be measured in Phase 5);
- cancels the engine's work if the client disconnects or presses Stop;
- records per-device usage counts (requests, tokens in/out, GPU seconds) in SQLite. **Prompt and reply text is never stored.** These counts are the starting point for billing if renting is added later.

### 6.9 Client (Ubuntu and Windows)

- **Device list.** Cards for each host with name, status badge (Ready, In use, Paused), and its models. Paired and unpaired hosts are shown separately.
- **Chat.** Streaming replies, a Stop button, markdown and code formatting, and a model picker. Switching models shows "Loading model on [host]... about 10 seconds".
- **Waiting in line.** When other friends' questions are ahead, the chat shows the client's place in line ("You're 2nd in line") and a Cancel button. It never shows whose questions are ahead or what they asked.
- **Graceful pauses.** If the host pauses mid-chat, the chat shows the reason and offers to retry when the host is available again.
- **History.** Stored in SQLite on the client. Nothing is kept on the host.
- **Local API.** An OpenAI-compatible address on localhost that forwards to the chosen host, so editors and scripts on the client can use the host's GPU too.

### 6.10 Packaging and updates

| | Windows (host and client) | Ubuntu (host and client) |
|---|---|---|
| Package | NSIS installer, per-user install | `.deb` (22.04 and later) and AppImage |
| Extra requirements | NVIDIA driver (already installed on the host) | WebKitGTK 4.1 (pulled in by the `.deb`). To share an NVIDIA card: the NVIDIA driver, with its Vulkan support |
| Tray icon | Native | AppIndicator (supported on stock Ubuntu) |
| Engine | `llama-server` CUDA build plus the CUDA runtime, several hundred MB, downloaded and checked on first run | `llama-server` Vulkan build, about 30 MB, downloaded and checked the first time sharing is switched on, so computers that only use never fetch it |
| Firewall | Rule for private networks added by the installer | Only needed if `ufw` is enabled; the Share screen gives the commands |

### 6.11 Ubuntu hosts

Added in v0.4 so the app can be tried Ubuntu to Ubuntu. The host works the same way as on Windows, with these differences:

- **Where the model runs.** With an NVIDIA card, on the card through the Vulkan build. Without one, on the processor only (`--device none`, so integrated graphics are never used): expect a few words a second, and pick a small model. The catalog includes Qwen3 0.6B and 1.7B for this. The memory limit is then measured against the computer's RAM, and the temperature and "another app is using the GPU" rules don't apply.
- **"Pause while I'm using the computer"** reads the idle time from GNOME (Ubuntu's standard desktop) over D-Bus. Other desktops don't offer it reliably; there the rule has no effect and the Limits screen says so.
- **Another app on the GPU** comes from the NVIDIA driver's per-process utilization. The desktop itself (GNOME Shell, Xorg, Xwayland) doesn't count, and neither does video playback, which mostly uses the decoder.
- **Staying awake** while answering uses a logind sleep inhibitor, released as soon as the answer is done.
- **The engine** runs at a lower priority (`nice` 10) and is killed if the app dies, the counterpart of the Windows Job Object.
- **Closing the window** keeps the app in the tray while sharing is on; otherwise it quits.

**Updates.** Tauri's auto-updater with signed update packages, so engine security fixes reach hosts without them doing anything.

**Code signing.** Without a code-signing certificate, Windows shows a "Windows protected your PC" warning on install. For the pilot, the friend can be told to expect it. A certificate is needed before any wider release.

**Protocol versioning.** A shared Rust module defines every message and API type used by both modes. Every message carries a protocol version so older and newer installs can tell when they're incompatible.

---

## 7. File safety

The owner's main concern is the safety of his files. The answer is that the app is built so the network has no route to them.

| The app can | The app cannot |
|---|---|
| Run AI models from its own models folder | Browse, open, read, upload, or change any other file |
| Receive chat requests from devices the owner paired | Show the owner's screen or let anyone control the PC |
| Download models the owner chose from the catalog | Run commands or programs sent from the network |
| Record how much each paired device used the GPU | Store the text of anyone's prompts or replies |

How this is enforced:

- The gatekeeper exposes only the five endpoints in section 6.8. There is no file, shell, or remote-desktop endpoint to misuse.
- The engine listens only on localhost, so the network can't reach it directly.
- The engine runs as the normal user (no admin rights) inside a Job Object, and features that write files are disabled.
- Unpaired devices can see the host's name and status, and nothing else.
- Paired friends never see each other's questions or answers, and the pause reasons they see don't name the owner's programs.
- Encrypted connections with pinned certificates stop other devices on the Wi-Fi from listening in or pretending to be a paired device.

Honest limits: no software is risk-free. A bug in the gatekeeper or engine could still be found. Keeping the exposed surface this small, and updating automatically, keeps the worst realistic outcome at "someone used the GPU", not "someone read the files".

It works the other way too: prompts are processed on the host's machine. The app doesn't store them, but a user who wouldn't want the host to see a prompt shouldn't send it.

---

## 8. Build plan (UI first)

The build starts with the user interface so that non-technical friends can see and click through how the finished app will work, and shape it, before the technical parts exist. Each later phase replaces a piece of pretend behavior with the real thing, without redesigning the screens.

| Phase | Goal | What friends see | Done when |
|---|---|---|---|
| 1 | Clickable prototype | Every screen, working with pretend data | Friends have tried it and the UI changes are agreed |
| 2 | Real chat | The same chat screen, now answering for real | Chat streams through the real gatekeeper and engine |
| 3 | Discovery | Their PC showing up live on the client's screen | Devices appear and disappear correctly on the home network |
| 4 | Pairing | The "wants to use your GPU" popup, for real | Only paired devices can chat |
| 5 | Limits | The settings they approved in Phase 1, actually working | Every pause rule works on the friend's laptop |
| 6 | Model library | Browsing, downloading, and removing real models | Download, verify, and "fits your GPU" work end to end |
| 7 | Installers and pilot | The real installer, then everyday use | Two-week pilot completed with no blocking issues |

### Phase 1: Clickable prototype

**Screens (host).** First-run welcome, tray menu, main Share screen with the switch and current status, resource-limit settings, model library (browse, fake download progress, remove), paired devices, the pairing popup, and "sharing paused because..." notices.

**Screens (client).** Device list with live-looking statuses, the pairing flow, chat with a streaming reply, model switching with a loading message, and the "host paused" message.

**How it's built.** The prototype uses the same web frontend the real app will use. All data comes from a mock layer that returns the same JSON formats described in section 6. Swapping mocks for real calls later then needs no redesign. The mock behavior stays honest: 8 GB limits, realistic model loading times, and pauses, so the preview never promises something the real app can't do.

**How it's shared.** As a web link that opens in any browser, with nothing to install. Every screen is labeled as a preview.

**Built:** `gpu-share-preview.html` (build 0.2). A single file, no install, no internet needed after the first load. It shows both machines side by side with a working simulation: the share switch, all the pause rules, the model library with downloads, pairing, a queue shared by several friends, and a chat that streams at the estimated speed of an RTX A2000. The test bench underneath fakes real life on the friend's laptop, including two other friends (Tunde and Ada) who can connect and ask questions so the queue can be tried.

**Feedback session with the friend.** Watch where he hesitates rather than asking whether he likes it. Questions to cover: are the setting names clear, are the defaults right, which apps should pause sharing, does he want notifications, and can he explain in his own words what the switch does and what the app can't do. With the other friends, check whether being told their place in line is enough, or whether they also want an estimate of how long the wait will be.

### Phase 2: Real chat end to end

Build the gatekeeper and engine supervisor and connect the chat screen to them. All of this is developed on the Ubuntu machine using a CPU build of `llama-server` and a tiny model, so no GPU is needed yet.

### Phase 3: Discovery

Real multicast announcements, heartbeats, the subnet-scan fallback, and "Add by IP". Tested on the actual home network with the friend's laptop running Share mode.

### Phase 4: Pairing and security

Per-install certificates, the pairing popup and codes, pinned connections, device tokens, and the paired-devices list.

### Phase 5: Limits wired up

The policy loop with real signals on the friend's laptop: idle timer, app watcher, GPU performance counters, NVML temperature and free VRAM, power status, and schedule. This is the first real GPU test. Record reply speed (tokens per second), model load time, VRAM use, and temperatures.

### Phase 6: Model library

The catalog file, resumable downloads, hash checks, GGUF header reading, and the VRAM estimate.

### Phase 7: Installers, updates, pilot

NSIS installer with the firewall rule, `.deb` and AppImage, the signed auto-updater, then a two-week pilot. Review the usage counts and the friend's experience at the end.

### Later

- **Remote access** over an embedded Tailscale connection, so sharing works outside the home network.
- **Several hosts**, with requests sent to whichever friend's PC is available.
- **Renting**: pricing, payments, per-device billing from the usage counts, and terms of use.

---

## 9. Risks and open questions

### Risks

| Risk | Mitigation |
|---|---|
| Laptop heat and fan noise | Temperature limit and "only when plugged in" on by default. Measure in Phase 5. |
| Router blocks multicast or device-to-device traffic | Subnet scan, "Add by IP", and a plain-language explanation in the app. |
| Windows network set to Public | First-run check that explains how to switch it to Private. |
| "Windows protected your PC" warning on install | Warn the pilot user in advance. Buy a code-signing certificate before any wider release. |
| Sharing competes with the owner's work for VRAM | Pause rules, idle unloading, memory budget. |
| One friend's long answers keep others waiting | Max reply length, one waiting request per device, place in line shown to each client. |
| Friends asking for different models cause repeated model swaps | Measure swap time in Phase 5. If it hurts, let the owner set one default model, or group waiting requests by model. |
| `llama-server` router mode keeps VRAM after idle | One engine process per model, stopped when not needed. |
| NVML can't report per-app VRAM on Windows | Use the Windows GPU performance counters instead. |
| The prototype sets expectations the real app can't meet | Mocks follow real limits and timings. Every screen labeled as a preview. |
| Laptop sleeps or loses power mid-chat | The client handles disconnects cleanly. Sharing starts off after a reboot by default. |
| llama.cpp changes quickly | Pin the engine version in each release and test before updating. |
| Large CUDA runtime download | Download on first run rather than inside the installer. |

### Open questions

- Project name.
- Frontend framework (Svelte or React).
- Where to host the model catalog and update files (GitHub Releases is the default option).
- Code-signing certificate: cost and timing.
- Which models go in the first catalog, after checking their licenses.
- Which apps to pre-fill in the pause list (ask the friend during Phase 1).
- Whether the owner should be able to give one friend priority, or set a daily limit per friend.
- How renting would work, if it happens.

Answers and their status are tracked in [roadmap.md](roadmap.md#decisions), not here.

---

## 10. Glossary

| Term | Meaning |
|---|---|
| GPU | Graphics card. Does the heavy maths for AI models. |
| VRAM | The graphics card's own memory. A model has to fit in it to run fast. The host has 8 GB. |
| Model | The AI "brain" file that answers questions. |
| GGUF | The file format llama.cpp uses for models. |
| Quantization (4-bit) | Shrinking a model so it fits in less memory, with a small drop in quality. |
| Token | A chunk of text, roughly three-quarters of a word. Speed is measured in tokens per second. |
| Context | How much of the conversation the model can keep in mind at once. Longer context uses more VRAM. |
| LAN | The local network, e.g. everyone on the same home Wi-Fi. |
| Host | The PC sharing its GPU. |
| Client | The PC using someone else's GPU. |
| Multicast | A way to send one message to every device on the local network at once. Used for discovery. |
| Gatekeeper | The part of the host app that decides which requests are allowed through. |
| Pairing | A one-time approval, like Bluetooth, that lets a specific device use the host. |
| Queue | The line of waiting questions when several friends ask at once. The host answers them one at a time, in order. |

---

## 11. References

- llama.cpp server documentation (router mode): <https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md>
- Model management in llama.cpp (Hugging Face blog): <https://huggingface.co/blog/ggml-org/model-management-in-llamacpp>
- llama.cpp issue #19379, idle router processes holding VRAM: <https://github.com/ggml-org/llama.cpp/issues/19379>
- llama.cpp RPC backend (considered for multi-PC splitting, not used): <https://github.com/ggml-org/llama.cpp/blob/master/tools/rpc/README.md>
- LocalSend protocol: <https://github.com/localsend/protocol>
- exo platform support: <https://github.com/exo-explore/exo/blob/main/PLATFORMS.md>
- GPUStack installation requirements: <https://docs.gpustack.ai/latest/installation/installation/>
- Tailscale device sharing (for future remote access): <https://tailscale.com/docs/reference/inviting-vs-sharing>
- LM Studio LM Link (similar idea, single-owner devices): <https://lmstudio.ai/link>

---

## Change log

| Version | Date | Changes |
|---|---|---|
| 0.1 | 11 September 2026 | First draft, based on the planning discussions. |
| 0.2 | 12 September 2026 | GPU confirmed as RTX A2000 8 GB. Phase 1 prototype built. |
| 0.3 | 9 October 2026 | Several friends can share one host: queue with place in line, one waiting request per device, model swap cost noted. Friends see general pause reasons, not the owner's open programs. Limits shown in tokens with approximate words. Client status labels aligned with the prototype. Prototype updated to build 0.2 to match. |
| 0.4 | 15 October 2026 | Ubuntu computers can host too (R8a, section 6.11), with an NVIDIA card or on the processor alone, for Ubuntu-to-Ubuntu testing. The engine is downloaded on first use on both platforms. |
