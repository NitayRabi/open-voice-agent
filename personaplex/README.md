# PersonaPlex-7B on the R9700

NVIDIA's full-duplex speech-to-speech model (Moshi architecture). This is an
*audio* model, not a text LLM -- it has nothing to do with the llama-swap /
llama.cpp stack on :8080 and does not go in models.ini.

## Status: running locally

The environment and weights are installed. Use `./run-webui.sh` for plain
PersonaPlex or `./run-webui-tools.sh` for the delegation POC.

## Layout

- `.venv/`        python 3.12 + torch 2.10.0+rocm7.0
- `src/`          github.com/NVIDIA/personaplex checkout
- `run-webui.sh`  live full-duplex web UI, HTTPS on :8998
- `run-offline.sh` WAV in -> spoken WAV reply, needs no microphone

## PersonaPlex + tool delegation POC

Run `./run-webui-tools.sh`. It starts both models on the R9700:

- PersonaPlex remains the full-duplex conversational voice model.
- Gemma E4B Q4 reads the latest Whisper user transcript plus PersonaPlex's
  inner assistant-text stream. It has one function: `delegate(message)`.
- The hardcoded `delegate` adapter currently invokes the local OpenClaw `main`
  agent. The returned spoken text is teacher-forced into PersonaPlex, so
  PersonaPlex says it in its own voice rather than switching to a TTS voice.

Ordinary conversation is ignored by the router. Requests for current/private
information, files, devices, apps, explicit delegation, or actions are routed.
The router endpoint/model and executor agent are configurable with
`PPLEX_ROUTER_URL`, `PPLEX_ROUTER_MODEL`, and `PPLEX_OPENCLAW_AGENT`.

Open `https://192.168.68.46:8998` and accept the self-signed certificate once.
The E4B endpoint is loopback-only on port 8011.

## Build notes (things that cost time)

**The system python's torch is `2.13.0+cu130` -- a CUDA build on an AMD box.**
It reports `cuda.is_available() == False` and is useless here. That is why this
lives in its own venv with the rocm7.0 wheels. Don't try to use system python.

**`moshi/pyproject.toml` pins `torch >= 2.2.0, < 2.5`.** That predates RDNA4
support entirely -- gfx1201 needs ROCm 6.4+/torch 2.7+. Installed with
`uv pip install --no-deps ./src/moshi/` and the other deps resolved by hand, so
the pin can't downgrade the ROCm build. Redo this the same way after any
`git pull` of src/.

**opus-devel is NOT needed** despite what the README says. `sphn` ships a
prebuilt wheel (0.1.12) with opus statically linked. Good, because sudo on this
box wants a password.

**`sounddevice` is skipped deliberately.** It is only used by the CLI client;
neither `moshi.server` nor `moshi.offline` imports it, so PortAudio is not
required.

**VRAM:** bf16 weights are 16.7 GB against the R9700's 31.9 GB. Fits with room
to spare -- `--cpu-offload` is not needed. `HIP_VISIBLE_DEVICES=0` pins it to
the R9700 so the 890M iGPU is never selected.

**Importing `moshi.server` executes `main()`** -- the module has a bare
top-level `main()` call with no `__name__` guard. Never `import moshi.server`
to probe it; run it as `-m`.

## THIS BOX HAS NO MICROPHONE

`wpctl status` shows zero audio Sources. The only sinks are Sonos speakers on
the network (Living Room, Bedroom, Era 100). A full-duplex voice model is
unusable locally as a result -- the browser has no capture device to offer.

Two ways around it:

1. **Talk from another device.** The web UI captures mic in the *client*
   browser, so open the UI from a phone or laptop. That is what `--ssl` is for:
   `getUserMedia` needs a secure context on any non-localhost origin. Certs are
   auto-generated with mkcert on first run; expect a trust warning to click
   through unless you install the mkcert CA on the client.
   - LAN:       https://192.168.68.46:8998
   - Tailscale: https://100.123.240.76:8998   (works off-network)
2. **`./run-offline.sh`** -- no mic involved at all. Streams
   `src/assets/test/input_assistant.wav` in and writes a spoken reply to
   `out/reply.wav` plus a transcript json. Use this to prove the model runs.

## Voices

NATF0-3 / NATM0-3 (natural), VARF0-4 / VARM0-4 (variety).
`VOICE=NATM1 ./run-offline.sh`. Persona is set by a text role prompt --
`--text-prompt "$(cat src/assets/test/prompt_service.txt)"`.
