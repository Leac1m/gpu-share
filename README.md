# GPU Share

Use a friend's idle graphics card over the home network, safely and on their terms.

- **What this is and why** → [docs/gpu-share-project.md](docs/gpu-share-project.md) — the full spec.
- **What's done, what's next, and open notes between agents** → [docs/roadmap.md](docs/roadmap.md) — read this before starting any task.
- **Clickable UI preview** → [docs/gpu-share-preview.html](docs/gpu-share-preview.html) — open directly in a browser, no build needed.

## Layout

```
crates/protocol/   Shared Rust types for every message the host and client exchange
apps/desktop/      Tauri 2 app: Svelte + TypeScript frontend over mock data, Rust side in src-tauri/
docs/              Spec, roadmap, and the HTML prototype
```

## Building

```
cargo test --workspace          # protocol crate tests
cargo clippy --workspace -- -D warnings

cd apps/desktop
npm install
npm run dev                     # or: npm run build
```

There is no GPU, network, or installer code yet — see [docs/roadmap.md](docs/roadmap.md) for what exists and what's next.
