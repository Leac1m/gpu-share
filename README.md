# GPU Share

Use a friend's idle graphics card over the home network, safely and on their terms.

- **What this is and why** → [docs/gpu-share-project.md](docs/gpu-share-project.md) — the full spec.
- **What's done, what's next, and open notes between agents** → [docs/roadmap.md](docs/roadmap.md) — read this before starting any task.
- **Clickable UI preview** → [docs/gpu-share-preview.html](docs/gpu-share-preview.html) — open directly in a browser, no build needed.
- **Trying it with friends** → [docs/pilot.md](docs/pilot.md) — installing on the owner's Windows laptop and friends' Ubuntu PCs, and what to check.
- **Making a release** → [docs/releasing.md](docs/releasing.md) — installers, and how to bump the pinned engine.
- **What the app can and cannot reach** → [docs/security-review.md](docs/security-review.md).

## Layout

```
crates/protocol/   Shared Rust types for every message the host and client exchange
crates/netcore/    Device identity and certificate-pinned TLS
crates/host/       Share mode: queue, engine supervisor, gatekeeper, usage counts
crates/client/     Use mode: talking to hosts, chat history
apps/desktop/      Tauri 2 app: Svelte UI (real backend in the app, mock in the browser preview)
docs/              Spec, roadmap, and the HTML prototype
scripts/           Cloud-session setup, dev engine build
```

## Building

```
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings

cd apps/desktop
npm ci
npm run dev                     # browser preview (mock backend) at http://localhost:5173
npm test && npm run build
```

To run the real app on a computer without a GPU, build a CPU engine and a tiny model first:

```
scripts/dev-engine.sh           # llama.cpp b11582 + SmolLM2-135M into .dev/
cd apps/desktop && npm run dev &  # the debug app loads its UI from the dev server
cargo run -p app                # Share mode appears because .dev/llama-server exists
```

See [docs/roadmap.md](docs/roadmap.md) for what is real so far and what's next.
