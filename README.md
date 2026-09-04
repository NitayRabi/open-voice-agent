# open-voice-agent

Local speech-to-speech voice agents on the R9700, delegating asynchronously to
OpenClaw. Two stacks were built; **VoiceChat is the one to use.**

## Desktop app

`app/` is a Tauri app that wraps the Hugging Face speech-to-speech approach in a
floating bubble: an always-on-top orb, a settings screen, a global hotkey to
toggle speech, mic capture + playback in AudioWorklets, and the OpenAI-Realtime
WS cascade (or the VoiceChat turn protocol). A conversational voice model does
the talking and has one tool, `delegate_task`, which hands real work to a single
more capable "brain" model behind an OpenAI-compatible endpoint and speaks back
its answer. It can also serve the same config + voice UI over HTTP(S) for a
browser or another device. No weights ship in the installer — the app downloads
GGUF models on demand (whisper.cpp-app style) or uses a remote endpoint. See
[`app/README.md`](app/README.md) to build and run it.

    ./run.sh                         # Q4_0, https://192.168.68.46:8999
    ./run.sh --quant Q8_0            # Q8_0
    ./run.sh --quant F16             # unquantized F16

Q4_0 is already installed on this machine. Prepare Q8_0 or F16 once with:

    ./prepare-models.sh Q8_0
    ./prepare-models.sh F16

## Layout

    run.sh                 launch VoiceChat + OpenClaw
    prepare-models.sh      download and split Q4_0 or Q8_0 assets
    voicechat/             the live stack
      vc_openclaw.py       --serve driver, tool bridge, hands-free web UI
      system.txt           system prompt (= the tool list; it is the warmup cost)
      turns/               per-turn in/out wav, kept for debugging
    personaplex/           superseded, kept for A/B. Owns the shared .venv.
      .venv/               python 3.12 + torch 2.10.0+rocm7.0 (both stacks use it)
      run-webui.sh         PersonaPlex alone
      run-webui-tools.sh   PersonaPlex + the Whisper/trigger-word bridge
      openclaw_bridge.py   that bridge

Deliberately NOT moved in here, because they match how the box is already
organised and are large:

  ~/llama-voicechat.cpp   engine fork, next to the other llama.cpp checkouts
  ~/models/voicechat      6.1 GB of GGUF, next to models.ini

Override with `VC_BIN` / `VC_MODELS` if either moves.

## Async tool calls

The web stack follows the separation used by Codex realtime voice: VoiceChat is
the live conversation frontend and OpenClaw is a background agent. When the
function head calls `ask_openclaw`:

1. the engine immediately receives a small "running in the background" tool
   response and VoiceChat acknowledges the delegation;
2. OpenClaw runs on a worker without blocking the model-output reader or the web
   request;
3. the completed result is passed through the resident VoiceChat TTS and pushed
   to every connected browser over an event stream;
4. the browser queues that handoff behind any audio already playing, then opens
   the microphone again.

The engine fork has one local protocol addition for step 3:

    {"cmd":"say","text":"...","out":"...wav"}

This is a POC handoff, not causal replay: the spoken OpenClaw result is not added
to the 11B model's conversational state. A production implementation should add
an engine-level context/handoff input or implement the frozen-context audio
replay used by fully asynchronous VoiceChat runtimes.

OpenClaw jobs are intentionally serialized (`max_workers=1`) so two long tool
calls cannot contend for the same agent or speak over each other.

## Quality A/B

`--quant` selects the STT/LLM, audio projector, and TTS together. The active
quantization is shown in the web UI and transcript labels. Model selection
happens at process start because each quant owns a different resident GPU model.

Q8 currently uses the known-good Q4 function-head sidecar. The converted Q8
sidecar loads, but did not emit a tool call for an explicit delegation request
that reliably triggers Q4. This does not change Q8 speech quality: the sidecar
only predicts the function-channel tokens. Set `VC_FUNCTION_QUANT=Q8_0` to
reproduce or continue investigating that behavior.

As a POC safety net, a user transcript that explicitly contains `OpenClaw` is
queued if the native function head emitted no call. The UI metadata identifies
this as `explicit_name_fallback`; requests that do not name OpenClaw still rely
on the model's native function decision.

    ./run.sh --quant Q4_0 --port 8999
    ./run.sh --quant Q8_0 --port 9000

Do not run both concurrently unless VRAM headroom has been checked. For a fair
subjective comparison, use the same microphone, prompt, system prompt, and fresh
session for each quant.

## Which stack, and why

| | PersonaPlex 7B | VoiceChat 11B |
|---|---|---|
| ms/frame (80 ms budget) | 83.6 -> RTF **1.04** | 47.9 -> RTF **0.60** |
| VRAM | 20.2 GB | **9.6 GB** |
| free alongside | 11.7 GB | **22.3 GB** (a local coding model fits) |
| tool calling | none; needed Whisper + a spoken trigger word | **native function head** |

PersonaPlex ran permanently 4.5% behind real time, which starved the browser's
jitter buffer -- that was the crackling, not the mic and not the model's audio
(both measured clean). Root cause was `other_mimi`, a second Mimi codec whose
encode and decode results are discarded every frame: 7.16 ms/frame of dead
compute against a 3.6 ms deficit. Removing it is bit-identical (same output
md5) and is patched in `personaplex/src/`.

## Two traps in llama-voicechat

**`--system-file` is silently ignored in `--serve` mode.** `main()` only calls
`run_system()` on the one-shot path. The symptom is not an error: the model
says it cannot do anything, which is TRUE because its tool list is empty, and a
one-shot `--tool-response` test passes because that path does apply the prompt.
Send `{"cmd":"system","text":...}` after `ready`; confirm via the `system_start`
event and its token count.

**`VC_NO_BARGE=1` + `VC_FORCE_BOS=1` are mandatory, together.** Without them the
model barges in ~1 s into the clip, answers only that first second, and every
later turn degenerates. So it is push-to-talk by nature; the UI does client-side
VAD auto-turns instead of true interruptible duplex.

## Where a tool turn's time goes (measured, 17.9 s turn)

    turn start        -> tool_call_start    3.57s   listening + deciding
    tool_call_start   -> tool_call          0.30s   writing the call
    tool_call         -> tool_response      3.85s   OpenClaw
    tool_response     -> tool_response_end  1.01s   splicing the result, 56 frames
    tool_response_end -> audio              8.25s   speaking, 51 frames @ 162 ms

Per-frame cost grows with context depth, and a tool turn is deep: system prompt
+ audio + call + result before it starts speaking. Hence the short system prompt
and the one-sentence tool result -- both are latency, not just tokens.

`PPLEX_DIAG=1` on PersonaPlex dumps per-turn input/output wav plus frame timings.
