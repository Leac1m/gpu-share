# Security review of the gatekeeper's exposed surface

Roadmap `P4-T7`. Reviewed 13 October 2026 by Claude (Opus 5.5) against spec §7 at commit
"Phase 4" on branch `claude/clever-noether-gv108e`. Every "cannot" in the spec's table has a test
or a structural argument below. Re-review whenever a route, an engine flag, or a file the app
writes is added.

## What is reachable from the network

| Listener | Bound to | Open when | Speaks |
|---|---|---|---|
| Gatekeeper | `0.0.0.0:50123` TCP | Sharing switch on (paused included) | HTTPS (rustls, TLS 1.2/1.3), HTTP/1.1 |
| Discovery | `0.0.0.0:53737` UDP, group `239.192.152.143`, TTL 1 | App running | JSON announcements |
| Owner's own listener (`P9-T1`) | `127.0.0.1:<random>` TCP | Share mode available (sharing on or off) | HTTPS, same as the gatekeeper; only `/api/info`, `/v1/models`, `/v1/chat/completions` |
| Engine (`llama-server`) | `127.0.0.1:<random>` TCP | A model is loaded | HTTP, random API key per start |

The owner's own listener lets the owner's Use mode reach his own GPU. It binds the loopback
address, has no pairing or discovery routes, and its paired endpoints need all three of: a
loopback peer, this install's own certificate (whose private key never leaves the data folder),
and a token made fresh by each run of the app and held only in memory
(`tests/owner_use.rs::only_this_installs_certificate_with_the_token_gets_in`). A paired friend's
certificate and token don't open it. The owner's questions skip the pause rules and aren't
counted as anyone's usage; friends' requests through the main gatekeeper are unchanged.

Nothing else listens. The engine is not reachable from the network at all: it binds the loopback
address, and its API key is generated fresh for every start and never leaves the host process
(`tests/supervisor.rs::starts_on_localhost_with_a_fresh_key_and_reports_progress`).

## The spec §7 "cannot" table

### Browse, open, read, upload, or change any other file

* **Structural.** The gatekeeper's router is built from `protocol::gatekeeper::ENDPOINTS` and
  panics at startup on any path without a handler (`crates/host/src/gatekeeper.rs::router`). None
  of the five handlers takes a path, a file name or a URL from the request.
* **Test (suite 3).** `tests/gatekeeper_surface.rs::only_the_five_endpoints_exist`: 35 plausible
  paths and methods (llama-server's own endpoints such as `/slots`, `/props`, `/completion`, plus
  `/api/files`, `/api/shell`, path traversal and case variants) all return 404.
* **The model field is never a path.** Chat requests name a model; the host looks the name up in
  the catalog of installed, enabled models and builds the file path itself from the catalog id
  (`HostCore::model_spec`). `tests/chat_flow.rs::unknown_models_are_refused_before_queueing`
  sends `"../model-a"` and gets `unknown_model`; `clamp.rs` rewrites the forwarded `model` to the
  catalog id (`the_model_field_is_the_hosts_alias_not_the_clients_string`).
* **The engine writes nothing.** `--slot-save-path` is never passed (checked in
  `tests/supervisor.rs`), `--no-slots` disables the slot endpoint, `--offline` stops it fetching
  anything. Request clamping rebuilds the forwarded body from an allowlist, so llama-server
  parameters that touch slots or files (`id_slot`, `slot_save_path`, `cache_prompt`…) cannot be
  smuggled through (`clamp.rs::engine_specific_parameters_never_reach_the_engine`).
* **The engine's file path doesn't leak.** llama-server reports the model's file path (which
  contains the owner's user name) in every chunk; the gatekeeper overwrites `model` and drops
  `timings` / `system_fingerprint` (`tests/chat_flow.rs::the_engines_file_path_never_reaches_the_client`).

### Show the owner's screen or let anyone control the PC

* **Structural.** No code captures the screen or synthesizes input. A grep for `SendInput`,
  `BitBlt` and process spawning shows the only spawned process is the engine binary, with
  arguments built in `engine.rs` from the catalog and the owner's settings.

### Run commands or programs sent from the network

* **Structural.** The only process the app starts is `llama-server`, from a path decided at
  startup (bundled resource, env override, or the dev build). Its arguments come from the catalog
  and the owner's settings, never from a request.
* **The catalog is fetched from this repository's `catalog` release** (`P6-T1`), so it is checked
  like untrusted input: a catalog entry may only add `--chat-template-kwargs` with a JSON object,
  `--jinja`, or a sampling flag with a number; anything else (`--host`, `--port`, file paths,
  `--ctx-size`, `-ngl`, …) makes the whole entry be skipped
  (`library::tests::a_catalog_can_only_pass_a_few_harmless_engine_flags`). Downloads stay limited
  to huggingface.co over HTTPS with a SHA-256.
* **Engine-side features** that could be abused (grammars, `n`, logprobs, `n_predict`) are not in
  `ChatRequest`, so they are dropped on parse (`protocol::api` test
  `unknown_request_fields_are_dropped_on_parse`).

### Store the text of anyone's prompts or replies

* **Test (suite 5).** `tests/no_prompt_text_stored.rs` runs chats carrying a magic string (full
  replies and one stopped part way), then reads every file the host wrote and fails if the
  string appears anywhere. The owner's view (`HostView`) is checked too.
* **The owner's own chats** (`P9-T1`) are kept in his own chat history (`client.db`), like any
  computer's Use mode keeps its own. That is his text on his computer; friends' text still never
  reaches a file.
* **Structural.** The host database (`store.rs`) has no text column that could hold a prompt.
  Engine output is kept in a 40-line ring buffer in memory and never logged; engine errors carry
  only the exit status (`engine.rs`). No `log::` call in the workspace formats a request body.

### Unpaired devices see name and status, nothing else

* **Test (suite 4).** `tests/auth_matrix.rs::the_access_matrix`: five kinds of caller (no
  certificate, unpaired, paired, paired with the wrong token, removed) × five endpoints.
  `/v1/models` and `/v1/chat/completions` return 401 for everyone but a paired device with its
  own token. `/api/info` returns name, status, OS, app and protocol version, GPU name and the
  general pause reason, and no model list.
* **Tokens are bound to certificates.** A paired request needs the device's certificate (checked
  on the TLS connection) *and* its token; a stolen token from another certificate is refused
  (`gatekeeper_surface.rs::a_stolen_token_from_another_certificate_is_useless`). Tokens are
  stored hashed (`store.rs::tokens_are_stored_hashed`).

### Paired friends never see each other's questions, and pause reasons don't name programs

* **Structural.** Each chat response streams only that request's engine output. Queue events
  carry a position number and nothing else (`protocol::api::QueueEvent`, golden fixture).
* **Test.** `chat_flow.rs::waiting_clients_are_told_their_place_and_take_turns` checks the owner's
  view lists who waits, not what they asked; `a_pause_turns_away_everyone_waiting_and_cuts_the_running_reply`
  checks friends get the general reason ("They're using their computer") while the owner's
  reason ("Blender is open") stays on the host.

### Encrypted, pinned connections

* **Tests.** `netcore::tls` tests: the right pin connects and the host learns the client's id; a
  different host certificate is refused. `tests/pairing.rs`:
  * `a_proxy_claiming_to_be_the_host_is_not_added`: Add by IP records the certificate that
    answered and refuses a host whose claimed id doesn't match it.
  * `a_man_in_the_middle_shows_different_codes`: through a TLS-terminating proxy the owner's
    popup and the friend's screen show different 6-digit codes, because each side derives the
    code from the fingerprints it actually sees (`protocol::pairing::pairing_code`).
* **Nothing is granted before Allow**: `pairing.rs::allow_pairs_the_device_and_both_screens_show_the_same_code`
  checks no device is stored while the popup is up.

### The engine runs as the normal user in a Job Object

* **Structural.** The app never asks for elevation (per-user NSIS install, `P7-T1`). On Windows
  the engine is assigned to a Job Object with kill-on-close and started with below-normal
  priority (`jobobject.rs`, `engine.rs`). **Not yet verified on Windows**; it's on the manual
  checklist in the roadmap.

## Denial of service and nuisance

| Risk | Mitigation | Test |
|---|---|---|
| A friend floods the queue | One waiting or running request per device | `chat_flow.rs::one_waiting_request_per_device` |
| Very long answers | `max_tokens` capped at the owner's limit | proptest in `clamp.rs` |
| Huge requests | 1 MB body limit, 200,000 characters of messages | `gatekeeper_surface.rs::an_oversized_body_is_refused` |
| Slow or idle connections | 10 s TLS handshake timeout, 10 s header timeout, at most 64 connections | — |
| Pairing popups from strangers | One popup at a time; a device turned away must wait 60 s | `pairing.rs` unit tests |
| Spoofed announcements | Clients pin the announced id; an impostor's address fails TLS, and the real host's next announcement or poll corrects the entry | `netcore::tls` tests |

## Residual risks (accepted for the pilot)

1. **llama-server and GGUF parsing bugs.** The engine parses model files and prompts. Models come
   only from the catalog (bundled, or published from this repository) or a Hugging Face link the
   owner pastes, with SHA-256 checks (`P6-T4`), and the engine version is pinned (`P7-T8`);
   updates are the remaining defence. Keep the engine current. Whoever can publish the `catalog`
   release can offer any Hugging Face file; the owner still has to press Add, and only the
   allowed engine flags above apply.
2. **Popup spam with fresh identities.** The 60 s cooldown is per certificate, and certificates
   are free to make, so a determined device on the Wi-Fi can keep putting popups up, one at a
   time. The owner can switch sharing off. Revisit if it happens in the pilot.
3. **Spoofed names in the device list.** Anyone on the LAN can announce a name and a status for
   an id it doesn't own. It can't be paired with or chatted to (TLS pins the id), but it can sit
   in a friend's list until the real host's announcement replaces it.
4. **Device tokens on the client** are stored in the client's own SQLite file in the user's app
   data folder, readable by that user. Anyone who can read the friend's files and also run code
   with the friend's certificate key can use the host.
5. **The webview.** Model replies are rendered as markdown; `src/lib/markdown.ts` escapes all
   input first (vitest) and the app's CSP allows no remote scripts. Device names from the
   network are rendered as text, never HTML.
6. **Prompts are seen by the host machine's memory and GPU.** As spec §7 says: the app doesn't
   store them, but a friend who wouldn't want the owner's computer to process a prompt shouldn't
   send it.
