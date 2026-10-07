# Open Voice Agent

A local, low-latency speech-to-speech voice agent that lives in a floating desktop bubble.

Open Voice Agent splits conversational voice interaction from heavy reasoning: a fast, lightweight conversational model runs on the real-time audio hot path, while complex tasks (coding, lookups, plans, calculations, system actions) are delegated asynchronously to a more capable **Brain** or coding agent over the [Agent Client Protocol (ACP)](https://agentclientprotocol.com).

Named agent profiles allow the same voice session to delegate to different backends (Claude Code, Codex, Gemini CLI, OpenClaw, remote OpenAI-compatible endpoints, or local GGUF models) without reconnecting or restarting the audio pipeline. You can run Open Voice Agent as a native desktop floating orb, serve it over HTTP(S) to any browser on your local network, or connect from a paired Android phone or Wear OS smartwatch.

---

## Table of Contents

- [Architecture & The Funnel](#architecture--the-funnel)
- [Repository Layout](#repository-layout)
- [Prerequisites](#prerequisites)
- [Building from Source](#building-from-source)
  - [1. Desktop Application (`app/`)](#1-desktop-application-app)
  - [2. Standalone Speech Cascade (`hf-s2s/`)](#2-standalone-speech-cascade-hf-s2s)
  - [3. Android & Wear OS Clients (`android-client/`)](#3-android--wear-os-clients-android-client)
- [How to Use & Configuration](#how-to-use--configuration)
  - [First-Run Setup Wizard](#first-run-setup-wizard)
  - [Desktop Controls & Global Hotkey](#desktop-controls--global-hotkey)
  - [Agent Profiles & Delegation Options](#agent-profiles--delegation-options)
  - [Speech Engines (STT & TTS)](#speech-engines-stt--tts)
  - [Web UI & Remote Access](#web-ui--remote-access)
- [Testing & Quality Assurance](#testing--quality-assurance)
- [Design Background: Why This Architecture?](#design-background-why-this-architecture)
- [Contributing](#contributing)
- [License](#license)

---

## Architecture & The Funnel

Everything in Open Voice Agent follows a single design rule: **narrow the task in front of you until only the next decision is left**.

### 1. The Setup Funnel (4 Screens)
First run presents a guided setup wizard shared by the desktop app and browser clients:
1. **Welcome** & system checks
2. **Conversational Model**: select a recommended catalog GGUF, provide your own local file, or connect to a remote OpenAI-compatible endpoint
3. **Delegation**: choose your brain backend (ACP coding agent, remote API, or local model)
4. **Review & Start**: validates configuration and starts the voice engine automatically

### 2. The Conversation Funnel (One Tool)
The conversational voice model is kept small and fast to stay ahead of real-time audio. It has a single tool: `delegate_task(request)`.
- When you ask a factual question, request a plan, or ask for code/actions, the voice model calls `delegate_task`.
- The application immediately acknowledges verbally (*"on it"*), while the request routes off the critical path to the **Brain**.
- The voice conversation remains active. When the Brain returns its answer, the conversational model speaks the result.

```text
 ┌───────────────────────┐
 │ Desktop Orb / Browser │ ◄─── mic & playback ───► ┌──────────────────────────────────────────────┐
 │ Android / Wear OS     │                          │ Speech-to-Speech Cascade (OpenAI-Realtime WS)│
 └──────────┬────────────┘                          │                                              │
            │ WebSocket                             │  VAD ──► STT ──► Conversational LLM ──► TTS  │
            ▼                                       │         (Parakeet/     (Gemma 4 /        (Qwen3/     │
 ┌───────────────────────┐                          │          Whisper)       LFM GGUF)         Kokoro)    │
 │ Tauri App / Webserver │                          └──────────────────────┬───────────────────────┘
 └──────────┬────────────┘                                                 │
            │ voice model calls delegate_task(request)                     │
            ▼                                                              │
 ┌────────────────────────────────────────────────────────┐                │
 │ Rust Delegation Supervisor                             │                │
 ├────────────────────────────────────────────────────────┤                │
 │ ├─► POST {brain_base_url}/chat/completions (vLLM/OpenAI)                │
 │ ├─► local llama-server on dedicated GGUF               │                │
 │ └─► acpx exec <agent> (Claude Code, Codex, Gemini, …)  │                │
 └──────────────────────────┬─────────────────────────────┘                │
                            │                                              │
                            └──── brain result spoken back to user ────────┘
```

---

## Repository Layout

| Directory / File | Description |
|---|---|
| [`app/`](app/) | **The desktop app & server core** — Tauri v2 application (Rust backend + vanilla TypeScript frontend, embedded static web server, model catalog manager, and delegation supervisor). |
| [`app/src/`](app/src/) | Frontend source files: desktop floating orb (`bubble.html`), responsive web client (`web.html`), settings/setup wizard (`index.html`), audio worklets (`worklets/`), and IPC/API bridge (`lib/`). |
| [`app/src-tauri/`](app/src-tauri/) | Rust backend: windowing, system tray, global hotkeys, model asset manager, ACP execution, and embedded HTTP/HTTPS server. |
| [`hf-s2s/`](hf-s2s/) | **Speech-to-Speech Engine** — Python cascade (VAD, STT, LLM, TTS) streaming over the OpenAI-Realtime WebSocket protocol, plus the ACP facade and delegation manager. |
| [`android-client/`](android-client/) | **Native mobile & wearable client** — Android phone app (`app/`) with floating foreground overlay and Wear OS companion app (`wear/`). |
| [`run.sh`](run.sh) | Root convenience script to launch the standalone speech-to-speech cascade (`hf-s2s/run-comparison.sh`). |

---

## Prerequisites

### General Hardware
- **GPU (Recommended for default models):** NVIDIA (CUDA) or AMD (ROCm/HIP) with ≥ 10 GB VRAM, or Apple Silicon Mac (M1/M2/M3/M4 with unified memory).
- **CPU Mode:** Supported using Kokoro TTS (`HF_S2S_TTS=kokoro`) and remote or small quantized models to minimize memory footprint.

### Software Requirements
- **Rust:** `≥ 1.77` with Cargo (`rustup update`).
- **Node.js:** `≥ 20.0` with `npm` (to build the TypeScript frontend).
- **Python:** `≥ 3.10` with `pip` and virtual environment support (for `hf-s2s/`; managed automatically on macOS).
- **Tauri CLI v2:** Installed via Cargo:
  ```bash
  cargo install tauri-cli --version '^2' --locked
  ```

### Platform-Specific Packages

#### Linux (Debian / Ubuntu)
```bash
sudo apt update
sudo apt install -y \
  build-essential pkg-config libglib2.0-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
  libayatana-appindicator3-dev librsvg2-dev libxdo-dev
```

#### Linux (Fedora)
```bash
sudo dnf install -y \
  gcc gcc-c++ pkg-config glib2-devel \
  webkit2gtk4.1-devel gtk3-devel libsoup3-devel \
  libappindicator-gtk3-devel librsvg2-devel libxdo-devel
```

#### Linux (Arch)
```bash
sudo pacman -S --needed \
  base-devel webkit2gtk-4.1 gtk3 libsoup3 \
  libappindicator-gtk3 librsvg xdotool
```

> **Building on Linux without root or missing `-devel` packages?**
> Use the included sysroot script to download the development closure into your user cache:
> ```bash
> cd app
> ./scripts/setup-build-sysroot.sh
> source ./scripts/build-env.sh
> ```

#### Apple Silicon macOS
- Install Xcode Command Line Tools:
  ```bash
  xcode-select --install
  ```
- No manual Python or MLX setup is required before building — the app automatically provisions its runtime dependencies (`uv`, `speech-to-speech`, and `llama-server`) on first run.

---

## Building from Source

### 1. Desktop Application (`app/`)

The desktop frontend is written in TypeScript and compiles directly to `dist/`, which is embedded into the Rust binary at compile time.

#### Install frontend dependencies & build static assets
```bash
cd app
npm ci
npm run build
```

#### Run in development mode (with hot-reload)
```bash
# From app/ directory
cargo tauri dev
```

#### Build release binary or installers
```bash
# Package into distribution bundles (.deb, .rpm, .AppImage, or .dmg):
cargo tauri build

# Or build the standalone binary directly:
npm run build
cd src-tauri
cargo build --release
# Binary will be at app/src-tauri/target/release/open-voice-agent
```

#### Apple Silicon macOS Build
```bash
cd app
npm ci
rustup target add aarch64-apple-darwin
cargo tauri build --target aarch64-apple-darwin
```
The resulting `.app` bundle and `.dmg` will be placed in `app/src-tauri/target/aarch64-apple-darwin/release/bundle/`.

---

### 2. Standalone Speech Cascade (`hf-s2s/`)

The speech engine can be run standalone without the Tauri desktop UI for headless servers, backend testing, or containerized environments.

#### Start the cascade
```bash
# Launch via the root helper script:
./run.sh

# Or directly:
bash hf-s2s/run-comparison.sh
```
The server exposes an OpenAI-Realtime compatible WebSocket endpoint at `ws://127.0.0.1:8766/v1/realtime` and an HTTP facade at `http://localhost:9000/`.

#### Useful environment variables for `run.sh`
- `HF_S2S_STT=whisper-turbo` — Use Whisper Large v3 Turbo instead of Parakeet (recommended for Hebrew and multilingual speech).
- `HF_S2S_TTS=kokoro` — Use Kokoro 82M TTS on CPU instead of GPU Qwen3-TTS (drastically reduces VRAM usage).
- `HF_S2S_KOKORO_THREADS=8` — Number of CPU threads dedicated to Kokoro synthesis.
- `HF_S2S_PORT=8766` — Custom WebSocket server port.

---

### 3. Android & Wear OS Clients (`android-client/`)

The mobile and wearable clients connect to a running desktop or server node over the network. They do not run local models.

#### Prerequisites
- JDK 17
- Android SDK 35 (`export ANDROID_HOME=/path/to/android-sdk`)

#### Build APKs
```bash
cd android-client

# Build Phone APK:
./gradlew :app:assembleDebug
# Output: android-client/app/build/outputs/apk/debug/app-debug.apk

# Build Wear OS Companion APK:
./gradlew :wear:assembleDebug
# Output: android-client/wear/build/outputs/apk/debug/wear-debug.apk
```

---

## How to Use & Configuration

### First-Run Setup Wizard
When you open Open Voice Agent for the first time, it guides you through a four-step configuration:
1. **Welcome**: Verifies local environment and permissions.
2. **Conversational Model**: Choose a small model responsible for real-time conversation:
   - **Gemma 4 E4B Instruct Q4_0** (Recommended, ~4.6 GB download).
   - **Liquid AI LFM2.5 1.2B Q4_K_M** (Smallest, ~730 MB download).
   - Or enter a custom local GGUF path or remote OpenAI-compatible endpoint.
3. **Delegation (The Brain)**: Select the intelligence layer that solves complex queries.
4. **Review & Launch**: Starts the managed voice runtime and presents the floating orb.

### Desktop Controls & Global Hotkey
- **The Floating Orb**:
  - Drag anywhere on your screen. Frameless, transparent, always-on-top.
  - Orb color indicates state: Idle (gray/dim), Connecting (yellow), Listening (green), Speaking (blue), Delegating (purple), Error (red).
  - Click the orb to start or stop listening.
  - Hover and click the gear icon to open **Settings**.
- **Global Hotkey**:
  - Default: `Ctrl+Shift+Space` (Linux) / `Cmd+Shift+Space` (macOS).
  - Toggles microphone capture on and off instantly from any application. Customizable in **Settings → General**.

### Agent Profiles & Delegation Options
In **Settings → Delegation**, you can configure one or more named agent profiles:
- **Remote OpenAI-Compatible Endpoint**: Point to OpenAI (`https://api.openai.com/v1`), OpenRouter, vLLM, or any compatible endpoint with an API key and model name (e.g. `gpt-4o`, `claude-3-7-sonnet`).
- **Local Model via `llama-server`**: Pick a downloaded GGUF model. The app manages a local `llama-server` instance automatically.
- **Coding Agents via ACP / `acpx`**:
  - Directly invoke agent CLIs you already use: Claude Code, Codex, Gemini CLI, Copilot, Cursor, OpenClaw, or custom ACP agents.
  - Automatically detects installed agents from your user environment.
  - Set tool permission boundaries: *Read-only* (safe default for autonomous voice tasks), *Full access*, or *No tools*.

### Speech Engines (STT & TTS)
Configure speech behavior in **Settings → Speech**:
- **Speech Recognition (STT)**:
  - *Parakeet TDT 0.6B* (Default): Ultra-low-latency transcription for English and European languages.
  - *Whisper large-v3-turbo*: Multilingual recognition with per-turn language detection (~200 ms additional latency; ideal for Hebrew and mixed-language input).
- **Speech Synthesis (TTS)**:
  - *Qwen3-TTS 0.6B* (Default): Expressive voice synthesis on GPU / MLX.
  - *Kokoro 82M TTS*: High-speed, lightweight synthesis running on CPU, leaving maximum VRAM free for local LLMs. Includes customizable English voices (e.g., Heart, Bella, Michael, Nicole).

### Web UI & Remote Access
In **Settings → Web**, enable the embedded web server to access Open Voice Agent from other devices on your LAN:
- **Address & Port**: Bind to `127.0.0.1` (local machine) or `0.0.0.0` (LAN access, default port `1730`).
- **Pairing Code**: Generates a one-time code to authenticate new devices.
- **Microphone & TLS**: Browsers require a secure context (HTTPS) to access the microphone from any origin other than `localhost`. Provide a TLS certificate and private key in the settings, or generate a self-signed certificate:
  ```bash
  openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem -days 365 -nodes -subj "/CN=$(hostname)"
  ```
- For protocol specifications and device token management, see [`app/docs/remote-access.md`](app/docs/remote-access.md).

---

## Testing & Quality Assurance

To ensure stability across all layers of the stack, run the corresponding test commands before submitting contributions:

### Frontend Typecheck & Build
```bash
cd app
npm ci
npm run typecheck
npm run build
```

### Rust Core Tests
```bash
cd app/src-tauri
cargo check
cargo test
```

### Python Speech & ACP Integration Tests
```bash
python3 -m unittest discover -s hf-s2s/tests -v
python3 -m py_compile hf-s2s/*.py
bash -n run.sh hf-s2s/run-comparison.sh
```

---

## Design Background: Why This Architecture?

Before arriving at this split architecture, two monolithic models were tested and benchmarked on identical hardware:

| Metric | PersonaPlex 7B | VoiceChat 11B | Open Voice Agent (Split Architecture) |
|---|---|---|---|
| **ms/frame (80 ms real-time budget)** | 83.6 ms (RTF **1.04** — behind real time) | 47.9 ms (RTF **0.60**) | **< 30 ms** |
| **VRAM Consumption** | 20.2 GB | 9.6 GB | **< 5 GB** for conversational stack |
| **VRAM Remaining for Reasoning** | 11.7 GB | 22.3 GB | **Ample room** for large brain models |
| **Tool Calling & Execution** | None (required external spoken trigger) | Native function head, but deep context caused 18s latency | **Instant verbal ack ("on it") + asynchronous delegation** |

### Key Takeaways:
1. **Real-time audio cannot wait for reasoning.** Monolithic models that attempt to converse and reason simultaneously suffer from severe latency spikes during tool turns because compute cost grows rapidly with context depth.
2. **The Funnel works.** By keeping the conversational model's prompt shallow and offloading complex tasks to an asynchronous Brain, response latency stays minimal and the conversational experience remains smooth and natural.

---

## Contributing

Contributions from the open source community are warmly welcomed!

1. **Issues & Discussions:** If you encounter a bug or have a feature proposal, please open a GitHub Issue with reproduction steps and system details.
2. **Submitting Changes:**
   - Create a feature branch (`git checkout -b feat/my-improvement`).
   - Ensure all tests pass (`npm run typecheck`, `cargo test`, and `python3 -m unittest`).
   - Open a Pull Request with a clear description of your changes.
3. **Coding Standards:**
   - Frontend is dependency-free vanilla TypeScript (no bundler complexity).
   - Rust follows standard `clippy` and `cargo fmt` formatting.
   - Respect user privacy: credentials and environment keys are stored locally with restricted permissions.

---

## License

Open Voice Agent is open-source software licensed under the [Apache-2.0 License](https://www.apache.org/licenses/LICENSE-2.0).
Individual models downloaded through the catalog retain their respective licenses (e.g. Gemma 4 E4B under Apache 2.0; Liquid AI LFM under the LFM Open License).
