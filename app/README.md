# Open Voice Agent — desktop app

A Tauri app that wraps the **Hugging Face `speech-to-speech`** approach
(VAD → STT → LLM → TTS, streamed over the OpenAI Realtime GA event set) in a
floating desktop bubble. Two models, cleanly split:

- a **conversational voice model** that just talks to you, and
- a **brain** — one more capable model behind an OpenAI-compatible endpoint.

The voice model has exactly one tool, `delegate_task`. When you want something
actually *done* (a calculation, a lookup, a plan, code, a decision), it hands the
task to the brain and speaks back the answer.

```
 bubble window ─┐                 ws://…/v1/realtime     ┌─ speech-to-speech serve ─┐
 browser  /     ├─ VoicePipeline ───────────────────────▶│  VAD · STT · LLM · TTS    │
 browser  :port ┘  mic + playback ◀──────────────────────│  Parakeet · LLM · TTS   │
        │           (same TS, either Tauri IPC or /api)  └──────────────────────────┘
        │ voice model calls delegate_task(request)
        ▼
   Rust `delegate` ──▶  POST {brain_base_url}/chat/completions   (llama.cpp / vLLM / OpenAI / …)
        │
        ▼  the brain's answer, spoken back into the conversation
```

## What's here

| Path | Role |
|---|---|
| `src/` | frontend sources — TypeScript, no bundler, `withGlobalTauri` |
| `src/bubble.html` · `bubble.ts` | the floating always-on-top orb (desktop) |
| `src/web.html` · `web.ts` | the same orb as a full-page web app (served over HTTP) |
| `src/index.html` · `settings.ts` | first-run setup and settings (Speech / Delegation / Models / Web / Log) |
| `src/lib/tauri.ts` | dual bridge — Tauri IPC in the app, `/api` + long-poll in a browser |
| `src/lib/pipeline.ts` | mic → realtime WS → speaker, VAD turns, tool-call delegation |
| `src/lib/types.ts` | the payload shapes shared with Rust (mirrors `config.rs` / `assets.rs`) |
| `src/worklets/` | 16 kHz mic capture + jitter-buffered playback (adapted from `huggingface/speech-to-speech`, Apache-2.0) |
| `dist/` | `tsc` output + copied HTML/CSS/assets — what Tauri and the web server serve (git-ignored) |
| `src-tauri/src/lib.rs` | windows, tray, global hotkey, commands |
| `src-tauri/src/brain.rs` | delegation: one call to an OpenAI-compatible `/chat/completions` |
| `src-tauri/src/assets.rs` | model download manager (resume, verify, progress) |
| `src-tauri/src/localbrain.rs` | supervises a local `llama-server` on a downloaded GGUF |
| `src-tauri/assets/catalog.json` | the bundled model catalog (URLs, sizes, licenses) |
| `src-tauri/src/webserver.rs` | embedded HTTP(S) server: static UI + JSON API + event long-poll |
| `src-tauri/src/backend.rs` | optional supervisor for the speech backend process |
| `src-tauri/src/config.rs` | settings, persisted to `~/.config/ai.openvoice.agent/config.json` |

## Features

- **Floating bubble** — frameless, transparent, always-on-top, drag to move,
  click to toggle speech, hover for the gear → Settings. Colour = state
  (idle / connecting / listening / speaking / thinking / delegating / error).
- **Global hotkey** — default `Ctrl/Cmd+Shift+Space`, editable in Settings.
  A press just emits `speech-toggle`; the bubble starts/stops the mic.
- **Required first-run setup** — shared by Tauri and browser clients. It selects
  a recommended local conversational model (or a remote OpenAI-compatible
  endpoint), configures delegation, validates the required fields, and starts
  the voice engine.
- **One speech pipeline** — Parakeet STT → a selectable conversational LLM →
  Qwen3-TTS over the OpenAI-Realtime event set. Backend processes, ports, and
  environment variables are implementation details.
- **Delegation** — when the voice model calls `delegate_task` the app
  acknowledges instantly ("on it"), calls the brain off the critical path, then
  feeds the answer back so the voice model speaks it. Toggle
  *speak the result* off for a bare ack.
- **Automatic engine lifecycle** — after setup, the app starts the fixed speech
  backend when it opens and stops it when it quits. Enable **Launch Open Voice
  Agent when I log in** to register the app with the OS; the managed backend then
  starts at login as well. `manage_backend`, `launch_command`, `launch_cwd`, and
  `launch_env` in `config.json` can also supervise a custom server command.
- **Models** — download GGUF weights on demand (nothing ships in the installer);
  see [What ships vs. what you download](#what-ships-vs-what-you-download).
- **Web UI** — the *Web* tab turns on an embedded server that serves the exact
  same UI over HTTP: the voice orb at `/`, the config screen at `/settings`.
  Same origin, one binary — `dist/` is embedded at compile time. The frontend auto-detects: Tauri IPC
  in the desktop windows, `fetch('/api/…')` + a `GET /api/events?since=` long-poll
  in a browser. Config and delegation are shared across every client.

### Serving the web UI

Settings → **Web**:

| field | notes |
|---|---|
| Bind address | `127.0.0.1` (this machine) or `0.0.0.0` (the LAN) |
| Port | default `1730` |
| Pairing code | browsers enter it once on the pairing screen and receive a secure HttpOnly cookie; API clients send it as `Authorization: Bearer`. The app can generate one when binding to `0.0.0.0`. |
| TLS cert / key | PEM paths. **Required for microphone access from anything but `localhost`** — browsers gate `getUserMedia` on a secure context. `openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem -days 365 -nodes -subj "/CN=$(hostname)"` |

Without TLS the config screen still works from any device; only the mic needs
HTTPS (or an SSH tunnel that makes it `localhost`). The tray has an **Open web
UI** item; the *Web* tab has an **Open web UI** button.

API surface (all also require the token if set):
`GET/POST /api/settings`, `POST /api/delegate {request}`, `GET /api/version`,
`GET|POST /api/backend/status|start|stop`, `GET /api/models` +
`POST /api/models/{add,download,cancel,remove,forget}`,
`POST /api/emit {event,payload}`, `GET /api/events?since=<cursor>` (long-poll),
everything else → static UI.

## What ships vs. what you download

The installer is ~4 MB. **No model weights, no runtimes, no Python.** It carries
only the binary, the embedded web UI, and a small model *catalog*
(`src-tauri/assets/catalog.json`).

Everything heavy is fetched by the app, into the OS app-data dir
(`~/.local/share/ai.openvoice.agent/models/` on Linux), or supplied by a remote
endpoint. Same pattern as the whisper.cpp desktop apps.

| component | bundled? | how you get it |
|---|---|---|
| app + web UI + icons + catalog | ✅ in the installer | — |
| **brain** LLM | ❌ | *Delegation → remote endpoint* (OpenAI / HF / your server), **or** *local* — download a GGUF from the **Models** tab and the app runs `llama-server` on it |
| speech STT + TTS (HF cascade) | ❌ | `speech-to-speech serve` auto-fetches them into the HF cache on first run |
| conversational LLM | ❌ | choose a catalog GGUF in first-run setup / *Models*, use an existing GGUF under **Advanced**, or configure a remote OpenAI-compatible endpoint |
| `llama-server` / `speech-to-speech` runtimes | ❌ | you install them; the app calls the binary/URL you point it at |

### Models tab

- **Gemma 4 E4B Q4_0** is **Recommended** and **Smallest** (4.59 GB download).
  **Qwen3.6 35B A3B Q4_K_M** is the larger alternative (22.29 GB).
- Both passed all eight intended delegation requests in the smoke test and use
  Apache 2.0, with no separate commercial license. This is a small routing test,
  not a general quality guarantee. See [selection evidence](docs/model-catalog.md).
- Downloads are restricted to this catalog, pinned to upstream revisions and
  SHA-256 hashes. They stream to `<id>.gguf.part` and resume via HTTP `Range`.
  Progress rides the `asset-progress` event; **Cancel** / **Delete** / **Forget**.
- No Hugging Face token is required. Optional authentication under **Advanced**
  can help with anonymous rate limits. Existing custom entries and local GGUF
  paths remain usable, but custom URL/repository downloads are no longer offered.

### Configuring the brain

*Remote* (default): any `/chat/completions` — `https://api.openai.com/v1` +
`gpt-4o-mini` + key, an HF Inference endpoint, or your own vLLM / llama-server.

*Local*: pick a downloaded model, set `llama-server` binary (not bundled —
install llama.cpp) + port/context/GPU-layers. The app starts/stops it and points
delegation at `http://127.0.0.1:<port>/v1`.

## Prerequisites

- Rust ≥ 1.77, a C toolchain.
- Node ≥ 20 + npm, to compile the TypeScript frontend.
- **Apple Silicon macOS:** Xcode command-line tools, Homebrew, `uv`, and
  llama.cpp: `xcode-select --install` then `brew install uv llama.cpp`. Intel
  Macs are not supported by the managed MLX speech stack.
- **Linux:** GTK 3, WebKitGTK 4.1, libsoup3 + `-devel`/`-dev` packages.
  Fedora: `sudo dnf install webkit2gtk4.1-devel gtk3-devel libsoup3-devel
  libappindicator-gtk3-devel librsvg2-devel libxdo-devel`.
  Debian/Ubuntu: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev
  libayatana-appindicator3-dev librsvg2-dev libxdo-dev`.
- Tauri CLI, for bundling: `cargo install tauri-cli --version '^2' --locked`.

### No root / no `-devel` packages? Use the local sysroot

`scripts/setup-build-sysroot.sh` downloads the `-devel` RPM closure with
`dnf download` (no root) into `~/.cache/ova-build-sysroot`:

```bash
./scripts/setup-build-sysroot.sh
source ./scripts/build-env.sh      # sets PKG_CONFIG_* / LD_LIBRARY_PATH / RUSTFLAGS
npm ci && npm run build            # the frontend, into dist/
cd src-tauri && cargo build --release
```

## Build & run

The frontend compiles with `tsc` into `dist/`, which Tauri serves and
`webserver.rs` embeds with `include_dir!`. `cargo tauri dev` / `cargo tauri build`
run that step for you (`beforeDevCommand` / `beforeBuildCommand`); a bare
`cargo build` needs it done first.

```bash
npm ci                               # once — installs TypeScript

cargo tauri dev                      # dev, webview hot reload
cargo tauri build                    # .deb / .rpm / .AppImage
# or just the binary:
npm run build && cd src-tauri && cargo build --release && ./target/release/open-voice-agent
```

### Apple Silicon macOS

The macOS launcher keeps the same pipeline and model split as Linux, but uses
the native backend for each component:

| component | Apple Silicon backend |
|---|---|
| Parakeet TDT 0.6B STT | `mlx-audio` on MPS |
| conversational GGUF (including Gemma 4 E4B) | llama.cpp on Metal |
| Qwen3-TTS 0.6B | `mlx-audio`, 6-bit by default |

Prepare the Python/MLX runtime once from the repository root, then build the
ARM64 app:

```bash
xcode-select --install
brew install uv llama.cpp
bash hf-s2s/setup-macos.sh

cd app
npm ci
rustup target add aarch64-apple-darwin
cargo tauri build --target aarch64-apple-darwin
```

The app and DMG are written below
`src-tauri/target/aarch64-apple-darwin/release/bundle/`. The build includes the
macOS microphone permission description, so macOS can prompt on first use.

Model weights and the Python runtime are deliberately not embedded in the app.
Keep the source checkout (including `.tmp/speech-to-speech`) where it was built;
a locally built app remembers that checkout even when opened from Finder. If
the checkout is moved later, set `launch_cwd` in
`~/Library/Application Support/ai.openvoice.agent/config.json` to its new
absolute path. `launch_cwd` must be the repository root, not `app/`.

The first engine start downloads the Parakeet and 6-bit Qwen3-TTS MLX weights,
so it can take several minutes. Logs remain visible in Settings. To trade voice
quality for lower memory use, put this in `launch_env` in the same config file:

```json
{
  "HF_S2S_TTS_MLX_QUANTIZATION": "4bit"
}
```

Homebrew installs `llama-server` at `/opt/homebrew/bin/llama-server`, which the
launcher checks explicitly because Finder apps receive a minimal `PATH`. A
custom build can be selected with `HF_S2S_LLM_BIN` in `launch_env`. The launcher
refuses Intel macOS instead of failing later inside MLX.

For the optional HTTPS/WSS LAN endpoint, also run `brew install socat` and
configure a TLS certificate and key in the Web settings. It is not needed for
the desktop app or a localhost browser.

An unsigned local build may need to be opened once with Control-click → Open.
Distributing it to other Macs still requires normal Apple code signing and
notarization, and recipients need a separately prepared speech runtime.

Frontend-only loops:

```bash
npm run build                        # clean dist/, copy static files, compile
npm run dev                          # the same, then tsc --watch
npm run typecheck                    # tsc --noEmit
```

First-run setup starts the speech pipeline automatically. For low-level
  development outside the app, it can still be launched directly:

```bash
bash hf-s2s/run-comparison.sh        # ws://127.0.0.1:8766/v1/realtime
```

## Verified

Boots under Xvfb: plugins load, tray registers, global hotkey registers, both
windows render, Tauri IPC round-trips, the bubble positions bottom-right and
shows the idle orb. With the web server on: a real browser loads `/` and
`/settings` (all tabs, no console errors), `GET/POST /api/settings` round-trips
to `config.json` on disk, `/api/events` long-poll delivers app events,
`POST /api/delegate` reaches the brain endpoint, and `POST /api/models/download`
fetches + verifies a real GGUF (tinyllamas, ~19 MB) with live `asset-progress`.
Not verified live: real audio, a running speech backend, and a multi-GB model
download (headless box; the models own the GPU).

## Known limitations

- The brain call is **non-streaming** (`stream: false`) — the answer arrives
  whole, then is spoken. Fine for one or two sentences; a long answer waits.
