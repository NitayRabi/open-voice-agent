# open-voice-agent

A local speech-to-speech voice agent that lives in a floating desktop bubble.
Talk to it; when you ask for something that needs real thinking, it hands the
work to a bigger model and speaks back the answer.

**[`app/`](app/) is the project.** It is a Tauri app wrapping the Hugging Face
`speech-to-speech` cascade (VAD → STT → LLM → TTS, streamed over the
OpenAI-Realtime GA event set): an always-on-top orb, a global hotkey, mic
capture and playback in AudioWorklets, and the same UI served over HTTP(S) to a
browser or another device. No weights ship in the installer — it downloads GGUF
models on demand or points at a remote endpoint.

Build and run it from [`app/README.md`](app/README.md).

Apple Silicon Macs are supported by the managed cascade: Parakeet and Qwen3-TTS
use MLX/MPS, while the conversational GGUF runs in llama.cpp on Metal. The
running app installs its own Python/MLX runtime and `llama-server` on first use
— no manual setup step before building; the complete macOS setup and build
instructions are in [`app/README.md`](app/README.md#apple-silicon-macos).

## The funnel

Everything that works about this project is one shape, applied twice: narrow the
thing in front of you until only the next decision is left.

**Getting started funnels to four screens.** First run is a guided setup, shared
by the desktop app and any browser client: welcome → pick the conversational
model (a catalog GGUF, your own file, or a remote OpenAI-compatible endpoint) →
configure delegation → review and start. It validates the required fields and
starts the voice engine at the end, so "installed" and "talking" are the same
event. No config file to find, no ports to reason about, no service to bring up
by hand.

**The conversation funnels to one tool.** The voice model is small and does
exactly one job: hold a conversation at real-time speed. It has a single tool,
`delegate_task`. When you want something actually *done* — a calculation, a
lookup, a plan, code, a decision — it calls that tool, the app acknowledges
instantly ("on it"), and the request goes to the **brain**: one more capable
model behind an OpenAI-compatible endpoint, off the critical path. The answer
comes back and the voice model speaks it.

```
 bubble window ─┐                 ws://…/v1/realtime     ┌─ speech-to-speech serve ─┐
 browser  /     ├─ VoicePipeline ───────────────────────▶│  VAD · STT · LLM · TTS    │
 browser  :port ┘  mic + playback ◀──────────────────────│  Parakeet · LLM · Qwen3   │
        │           (same TS, either Tauri IPC or /api)  └──────────────────────────┘
        │ voice model calls delegate_task(request)
        ▼
   Rust `delegate` ──▶  POST {brain_base_url}/chat/completions   (llama.cpp / vLLM / OpenAI / …)
        │
        ▼  the brain's answer, spoken back into the conversation
```

Splitting the models this way is what buys the latency. A voice model kept small
enough to stay ahead of real time can't also be the model that reasons; asking
one model to do both is what sank the earlier attempts below. The brain is
allowed to be slow because nothing is waiting on it — the conversation keeps
going while it runs.

## Layout

    app/          the Tauri desktop app — frontend (TypeScript, no bundler) + Rust
    hf-s2s/       the speech cascade the app supervises
      run-comparison.sh   launch it standalone: ws://127.0.0.1:8766/v1/realtime
      setup-macos.sh      one-time Python/MLX runtime prep for Apple Silicon

## Provider-neutral agent delegation

The supported standalone cascade can hand longer work to any Agent Client
Protocol (ACP) process without putting an agent harness on the audio hot path:

```text
microphone → Parakeet-TDT → Gemma E4B → Qwen3-TTS → speaker
                              │
                              └─ delegate_to_agent → ACP facade → provider
```

Run it with `./run.sh`, then open `http://localhost:9000/`. The tracked proxy
keeps ACP processes and sessions outside the speech server, starts delegations
as background jobs, shows their queued/running/succeeded/failed state, and
injects the final result into the live conversation so Gemma can speak it.

Providers are configured in [`hf-s2s/agents.json`](hf-s2s/agents.json) and can
also be managed in **Settings → ACP providers**. Each provider has a command,
arguments, working directory, timeout, optional environment, display name,
aliases, and routing guidance. The voice tool itself remains the generic
`delegate_to_agent({ task, agent? })`; it does not encode an OpenClaw- or
Codex-specific contract. Environment values are persisted locally with
owner-only permissions and the management API exposes only their names.

The default provider is the local OpenClaw Gateway agent “Milo”:

```bash
openclaw gateway status
openclaw acp --session agent:main:main
```

For a remote Gateway, edit the provider arguments to include `--url` plus
`--token-file` (preferred over putting a token directly in the registry).
OpenClaw must be available on `PATH` for the process that launches the voice
stack. Other ACP agents can be added alongside it and selected by ID, display
name, or alias.

ACP execution and provider mutation on the standalone proxy are loopback-only.
Phones, watches, and other LAN clients use the paired Tauri node, whose
authenticated `/api/delegate` route resolves the same default provider
registry. This keeps remote realtime access and OpenClaw execution behind the
node's pairing and per-device revocation boundary.

Validate the integration without loading the speech models:

```bash
python3 -m unittest discover -s hf-s2s/tests -v
python3 -m py_compile hf-s2s/*.py
bash -n run.sh hf-s2s/run-comparison.sh
```

Apple Silicon Macs run the same pipeline on native backends — Parakeet and
Qwen3-TTS on MLX/MPS, the conversational GGUF in llama.cpp on Metal. Run
`bash hf-s2s/setup-macos.sh` before building the ARM64 app; full instructions
are in [`app/README.md`](app/README.md#apple-silicon-macos).

## Android client

`android-client/` is a remote-only native client: pair it with the desktop
node's web URL and pairing code, then tap to talk. It never starts a local
server or model. An optional draggable foreground orb can sit over other apps
for one-tap voice access. See [`android-client/README.md`](android-client/README.md).

## What we tried first

Two earlier stacks were built on this box and both are gone as of this commit
(`git log` if you want them back). Neither is worth reviving, but the reasons
are worth keeping, because they are why the app looks the way it does.

| | PersonaPlex 7B | VoiceChat 11B |
|---|---|---|
| ms/frame (80 ms budget) | 83.6 → RTF **1.04** | 47.9 → RTF **0.60** |
| VRAM | 20.2 GB | **9.6 GB** |
| free alongside | 11.7 GB | **22.3 GB** |
| tool calling | none; needed Whisper + a spoken trigger word | native function head |

**PersonaPlex** ran permanently 4.5% behind real time, which starved the
browser's jitter buffer — that was the crackling, not the mic and not the
model's audio, both of which measured clean. Root cause was `other_mimi`, a
second Mimi codec whose encode and decode results were discarded every frame:
7.16 ms/frame of dead compute against a 3.6 ms deficit. Removing it was
bit-identical (same output md5). It still had no native tool calling.

**VoiceChat** was fast enough and did have a native function head, but it was
one model doing everything, and the engine fork it needed had sharp edges:
`--system-file` was silently ignored in `--serve` mode (the model would say it
could not do anything, which was true — its tool list was empty), and
`VC_NO_BARGE=1` + `VC_FORCE_BOS=1` were both mandatory or the model barged in a
second into every clip. It was push-to-talk by nature. A measured 17.9 s tool
turn spent 8.25 s just speaking the result, because per-frame cost grows with
context depth and a tool turn is the deepest context there is: system prompt +
audio + call + result, all before the first word comes out.

That last number is the whole argument for the funnel. Keep the speaking model
small and its context shallow, keep the system prompt short, keep the tool
result to one sentence, and push everything else to a model that is allowed to
take its time.
