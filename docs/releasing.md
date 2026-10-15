# Releasing GPU Share

## Cutting a release

1. Make sure CI is green on the commit you want to ship.
2. Set the version in `apps/desktop/src-tauri/tauri.conf.json` (`version`); the installers take
   their version from there.
3. Tag it and push the tag: `git tag v0.1.0 && git push origin v0.1.0`.
4. The **Release** workflow (`.github/workflows/release.yml`) builds:
   * the Windows installer (NSIS, per-user) with the engine manifest for the pinned llama.cpp
     release,
   * the Ubuntu `.deb` and AppImage (built on Ubuntu 22.04 so they also run on 24.04), with the
     engine manifest for the pinned llama.cpp release's Linux Vulkan build,
   and creates a **draft** GitHub release with them attached. Read it over and publish it.

To try a build without a tag, run the workflow by hand (Actions → Release → Run workflow) and
download the files from the run's artifacts.

The installers are not code-signed yet (roadmap `P7-T7`, decision D4), so Windows shows "Windows
protected your PC" the first time. See [pilot.md](pilot.md) for what to tell friends.

## Changing the model catalog (`P6-T1`)

The app's model list is `crates/host/catalog.json`. Release builds fetch the published copy
at start (`https://github.com/Leac1m/gpu-share/releases/download/catalog/catalog.json`), keep
it for offline starts, and fall back to the copy bundled in the app. So adding, changing or
removing a model doesn't need a new app release:

1. Edit `crates/host/catalog.json`. Pin `file_url` to a commit
   (`.../resolve/<commit>/<file>.gguf`), and take `size_bytes` and `sha256` from Hugging
   Face (the file's page, or `scripts/check-catalog.py`, which prints any mismatch).
2. Run `python3 scripts/check-catalog.py` and
   `cargo test -p host --lib library::tests::the_bundled_catalog_passes_validation`.
3. Merge to `main`. The **Catalog** workflow (`.github/workflows/catalog.yml`) runs the same
   checks and uploads the file to the `catalog` release (a prerelease, so it never shows as the
   app's latest release). Apps pick it up the next time they start.

Rules the app enforces on every entry, so a bad catalog can't do harm: the id is a plain file
name, the download is HTTPS from huggingface.co, the SHA-256 is 64 hex characters, and
`engine_args` may only contain `--chat-template-kwargs <json object>`, `--jinja`, and the
sampling flags `--temp`, `--top-p`, `--top-k`, `--min-p`, `--repeat-penalty`,
`--presence-penalty` with a number. Entries that break a rule are skipped.

Only add *optional* fields to entries: older apps read the same file. A model that needs a
newer engine than older apps have gets `"min_app_version": "x.y.z"`, and those apps leave it out.

## The engine version (`P7-T8`)

llama.cpp moves quickly, so the engine is pinned. `engine-version.txt` holds the llama.cpp
release tag; `scripts/dev-engine.sh` builds that tag for development, and the Release workflow
puts that tag's Windows CUDA build (and the matching CUDA runtime) in the engine manifest, with
SHA-256s, which the app downloads and checks on first run.

The app installs each engine version into its own folder (`<app data>/engine/<tag>/`), so a new
release with a new tag downloads the new engine once and never mixes files from two versions.

### Before bumping it

1. **Read the llama.cpp release notes** between the current tag and the new one for anything
   touching `llama-server`. GPU Share depends on:
   * flags: `-m`, `--host`, `--port`, `--api-key`, `--parallel`, `--ctx-size`, `--alias`,
     `--no-slots`, `--offline`, `-ngl`, `--device`, `--chat-template-kwargs` (Qwen3's thinking switch);
   * `GET /health` returning 503 while loading and 200 when ready;
   * streaming `POST /v1/chat/completions` with `stream_options.include_usage` sending a final
     chunk with `usage.prompt_tokens` / `usage.completion_tokens`;
   * the Windows release assets being named `llama-<tag>-bin-win-cuda-12.x-x64.zip` and
     `cudart-llama-bin-win-cuda-12.x-x64.zip`, and the Linux one
     `llama-<tag>-bin-ubuntu-vulkan-x64.tar.gz` with `llama-server` and its libraries in one
     folder (the workflow fails loudly if not);
   * `--device none` keeping a Linux host without an NVIDIA card on the processor.
2. **Change `engine-version.txt`**, delete `.dev/llama-server`, and run `scripts/dev-engine.sh`.
   Check `.dev/llama-server --help` still lists every flag above.
3. **Run the checks that use the real engine**:
   ```
   cargo test -p host --test real_engine -- --ignored
   scripts/e2e/run.sh eth0          # or your LAN interface
   scripts/e2e/installed.sh eth0    # the installed .deb, Ubuntu to Ubuntu, real engine + Qwen3 0.6B
   ```
4. **On the host laptop** (the RTX A2000): install a build from the Release workflow, let it
   download the new engine, and compare load time, reply speed and VRAM use with the numbers in
   the roadmap's Phase 5 notes (`P5-T11`). A big change in any of them is a reason to wait.
5. Note the bump in the roadmap's Phase 7 notes with the date and what you checked.
