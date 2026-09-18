#!/usr/bin/env bash
# Cascaded voice agent: Parakeet or Whisper -> local or remote conversational model -> Qwen3-TTS.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SHARED_ROOT="$ROOT"
if [[ ! -d "$SHARED_ROOT/.tmp/speech-to-speech" ]]; then
  COMMON_GIT_DIR="$(git -C "$ROOT" rev-parse --git-common-dir 2>/dev/null || true)"
  if [[ -n "$COMMON_GIT_DIR" && "$COMMON_GIT_DIR" != /* ]]; then
    COMMON_GIT_DIR="$ROOT/$COMMON_GIT_DIR"
  fi
  SHARED_ROOT="$(dirname "$COMMON_GIT_DIR")"
fi
UPSTREAM="${HF_S2S_UPSTREAM:-$SHARED_ROOT/.tmp/speech-to-speech}"
VENV="$UPSTREAM/.venv"
PLATFORM="$(uname -s)"
ARCH="$(uname -m)"
if [[ -d "$ROOT/hf-s2s/ssl" ]]; then
  default_ssl_dir="$ROOT/hf-s2s/ssl"
else
  # Existing development certificates live in the primary checkout. Worktrees
  # reuse them without copying private key material into each checkout.
  default_ssl_dir="$SHARED_ROOT/personaplex/ssl"
fi
SSL_DIR="${SSL_DIR:-$default_ssl_dir}"
# The desktop app overrides these with the exact cert and key it serves the
# web UI with, so the page and the wss endpoint present one identity and a
# browser only has to trust a certificate once. Standalone, they fall back to
# the development cert next to this repo.
SSL_CERT="${SSL_CERT:-$SSL_DIR/cert.pem}"
SSL_KEY="${SSL_KEY:-$SSL_DIR/key.pem}"
LAN_IP="${HF_S2S_LAN_IP:-192.168.68.46}"
BACKEND_PORT="${HF_S2S_BACKEND_PORT:-8766}"
WSS_PORT="${HF_S2S_WSS_PORT:-8765}"
UI_PORT="${HF_S2S_UI_PORT:-9000}"
LAN_UI_PORT="${HF_S2S_LAN_UI_PORT:-9001}"
LLM_PORT="${HF_S2S_LLM_PORT:-8011}"
if [[ "$PLATFORM" == "Darwin" ]]; then
  if [[ "$ARCH" != "arm64" ]]; then
    echo "The managed macOS speech stack requires Apple Silicon (arm64); found ${ARCH}." >&2
    exit 1
  fi
  # Finder-launched apps do not inherit the interactive shell's Homebrew PATH.
  # Prefer the standard Apple Silicon Homebrew location, while preserving an
  # explicit override and allowing other installations found on PATH.
  if [[ -x /opt/homebrew/bin/llama-server ]]; then
    default_llm_bin=/opt/homebrew/bin/llama-server
  else
    default_llm_bin="$(command -v llama-server || true)"
    default_llm_bin="${default_llm_bin:-llama-server}"
  fi
  default_llm_model="${HOME}/models/gemma-4-E4B_q4_0-it.gguf"
else
  default_llm_bin=/home/nitayrabi/llama.cpp/build-hip/bin/llama-server
  default_llm_model=/home/nitayrabi/models/gemma-4-E4B_q4_0-it.gguf
fi
LLM_BIN="${HF_S2S_LLM_BIN:-$default_llm_bin}"
LLM_MODEL="${HF_S2S_LLM_MODEL:-$default_llm_model}"
LLM_BASE_URL="${HF_S2S_LLM_BASE_URL:-}"
LLM_NAME="${HF_S2S_LLM_NAME:-local-conversation}"
LLM_API_KEY="${HF_S2S_LLM_API_KEY:-}"
TTS_MODEL="${HF_S2S_TTS_MODEL:-Qwen/Qwen3-TTS-12Hz-0.6B-CustomVoice}"
TTS_HIP_BIN="${HF_S2S_TTS_HIP_BIN:-$SHARED_ROOT/.tmp/qwen3-tts-hip/target/release/tts-server}"
TTS_HIP_MODEL_DIR="${HF_S2S_TTS_HIP_MODEL_DIR:-/home/nitayrabi/.cache/huggingface/hub/models--Qwen--Qwen3-TTS-12Hz-0.6B-CustomVoice/snapshots/85e237c12c027371202489a0ec509ded67b5e4b5}"
TTS_HIP_PORT="${HF_S2S_TTS_HIP_PORT:-8021}"

# Speech recognition. Parakeet is the fastest but only understands European
# languages. Whisper large-v3-turbo detects the language of every turn, so it
# handles Hebrew and Hebrew/English mixed speech for roughly 200 ms more per turn.
STT="${HF_S2S_STT:-parakeet}"
INIT_CHAT_PROMPT="You are a natural, concise voice assistant. Speak conversationally. Use delegate_to_agent for longer, agentic, coding, computer, or personal-automation tasks. When delegation starts, immediately say which agent started and that you will report back; never imply it already finished. When a delegation status update arrives, always speak its success or failure and briefly report the result. Do not delegate a status notification again."
case "$STT" in
  parakeet)
    if [[ "$PLATFORM" == "Darwin" ]]; then stt_device=mps; else stt_device=cuda; fi
    STT_ARGS=(
      --stt parakeet-tdt
      --parakeet_tdt_device "$stt_device"
      --parakeet_tdt_compute_type float16
    )
    ;;
  whisper-turbo)
    if [[ "$PLATFORM" == "Darwin" ]]; then
      STT_ARGS=(
        --stt mlx-audio-whisper
        --mlx_audio_whisper_model_name mlx-community/whisper-large-v3-turbo
        --language auto
      )
    else
      STT_ARGS=(
        --stt whisper
        --stt_model_name openai/whisper-large-v3-turbo
        --stt_device cuda
        --stt_torch_dtype float16
        --language auto
      )
    fi
    # Qwen3-TTS cannot speak Hebrew, so answer in English whatever was heard.
    INIT_CHAT_PROMPT="$INIT_CHAT_PROMPT Always reply in English, even when the user speaks another language."
    ;;
  *)
    echo "Unknown HF_S2S_STT '${STT}'; expected 'parakeet' or 'whisper-turbo'." >&2
    exit 1
    ;;
esac

children=()
cleanup() {
  for pid in "${children[@]:-}"; do
    kill "$pid" 2>/dev/null || true
  done
  for pid in "${children[@]:-}"; do
    wait "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT INT TERM

cd "$UPSTREAM"

if [[ ! -x "$VENV/bin/speech-to-speech" ]]; then
  echo "speech-to-speech runtime not found at ${VENV}." >&2
  if [[ "$PLATFORM" == "Darwin" ]]; then
    echo "Run: bash hf-s2s/setup-macos.sh" >&2
  fi
  exit 1
fi

if [[ -z "$LLM_BASE_URL" ]]; then
  if ! command -v "$LLM_BIN" >/dev/null 2>&1 && [[ ! -x "$LLM_BIN" ]]; then
    echo "llama-server not found at '${LLM_BIN}'." >&2
    if [[ "$PLATFORM" == "Darwin" ]]; then
      echo "Install it with 'brew install llama.cpp' or set HF_S2S_LLM_BIN." >&2
    fi
    exit 1
  fi
  "$LLM_BIN" \
    --model "$LLM_MODEL" --alias "$LLM_NAME" \
    --host 127.0.0.1 --port "$LLM_PORT" --ctx-size 4096 --parallel 1 \
    --gpu-layers all --flash-attn on --reasoning off --no-webui &
  llm_pid="$!"
  children+=("$llm_pid")

  for _ in $(seq 1 120); do
    if curl -fsS "http://127.0.0.1:${LLM_PORT}/health" >/dev/null 2>&1; then
      break
    fi
    if ! kill -0 "$llm_pid" 2>/dev/null; then
      echo "Local conversational model exited during startup" >&2
      exit 1
    fi
    sleep 0.5
  done
  LLM_BASE_URL="http://127.0.0.1:${LLM_PORT}/v1"
fi

if [[ "$PLATFORM" == "Darwin" ]]; then
  # The upstream macOS preset selects MLX/MPS for Parakeet and Qwen3-TTS.
  # Keep the conversational model on llama.cpp so the app can continue to use
  # its downloaded GGUF catalog and tool-capable Chat Completions adapter.
  "$VENV/bin/speech-to-speech" serve \
    --mac-optimal-settings \
    --host 127.0.0.1 --port "$BACKEND_PORT" \
    "${STT_ARGS[@]}" \
    --llm_backend chat-completions \
    --model_name "$LLM_NAME" \
    --responses_api_base_url "$LLM_BASE_URL" \
    --responses_api_api_key "$LLM_API_KEY" \
    --tts qwen3 \
    --qwen3_tts_model_name "$TTS_MODEL" \
    --qwen3_tts_device mps \
    --qwen3_tts_mlx_quantization "${HF_S2S_TTS_MLX_QUANTIZATION:-6bit}" \
    --qwen3_tts_speaker Aiden \
    --qwen3_tts_language auto \
    --stream_batch_sentences 1 \
    --init_chat_prompt "$INIT_CHAT_PROMPT" &
else
  LD_LIBRARY_PATH="/lib64:${LD_LIBRARY_PATH:-}" \
    "$TTS_HIP_BIN" "$TTS_HIP_MODEL_DIR" "127.0.0.1:${TTS_HIP_PORT}" 240 &
  tts_hip_pid="$!"
  children+=("$tts_hip_pid")

  for _ in $(seq 1 120); do
    if curl -fsS "http://127.0.0.1:${TTS_HIP_PORT}/health" >/dev/null 2>&1; then
      break
    fi
    if ! kill -0 "$tts_hip_pid" 2>/dev/null; then
      echo "Native HIP Qwen3-TTS exited during startup" >&2
      exit 1
    fi
    sleep 0.5
  done

  QWEN3_TTS_HIP_URL="http://127.0.0.1:${TTS_HIP_PORT}" "$VENV/bin/speech-to-speech" serve \
    --host 127.0.0.1 --port "$BACKEND_PORT" \
    "${STT_ARGS[@]}" \
    --llm_backend chat-completions \
    --model_name "$LLM_NAME" \
    --responses_api_base_url "$LLM_BASE_URL" \
    --responses_api_api_key "$LLM_API_KEY" \
    --tts qwen3 \
    --qwen3_tts_model_name "$TTS_MODEL" \
    --qwen3_tts_backend hip-http \
    --qwen3_tts_device cuda \
    --qwen3_tts_dtype bfloat16 \
    --qwen3_tts_attn_implementation eager \
    --qwen3_tts_speaker Aiden \
    --qwen3_tts_language auto \
    --qwen3_tts_parity_mode True \
    --stream_batch_sentences 1 \
    --init_chat_prompt "$INIT_CHAT_PROMPT" &
fi
backend_pid="$!"
children+=("$backend_pid")

for _ in $(seq 1 600); do
  if curl -sS "http://127.0.0.1:${BACKEND_PORT}/" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$backend_pid" 2>/dev/null; then
    echo "speech-to-speech backend exited during startup" >&2
    exit 1
  fi
  sleep 1
done

# The backend binds loopback and speaks plain ws. Everything a browser on another
# device can reach goes through this TLS wrapper, so without a usable cert there
# is nothing to wrap: say so loudly and skip it rather than leaving a plaintext
# listener, or one holding a certificate that does not match the page's.
if [[ -r "$SSL_CERT" && -r "$SSL_KEY" ]]; then
  socat \
    "OPENSSL-LISTEN:${WSS_PORT},reuseaddr,fork,cert=${SSL_CERT},key=${SSL_KEY},verify=0" \
    "TCP:127.0.0.1:${BACKEND_PORT}" &
  socat_pid="$!"
  children+=("$socat_pid")

  sleep 0.5
  if ! kill -0 "$socat_pid" 2>/dev/null; then
    echo "socat could not open wss://0.0.0.0:${WSS_PORT} with ${SSL_CERT}" >&2
    exit 1
  fi
else
  echo "No readable TLS cert/key (${SSL_CERT}, ${SSL_KEY})." >&2
  echo "Skipping the wss://:${WSS_PORT} wrapper and the LAN UI; only localhost can reach the agent." >&2
fi

# Same-machine endpoint: localhost is a browser secure context, so microphone
# access works without accepting a development certificate.
SPEECH_TO_SPEECH_URL="/v1/realtime" \
OPEN_VOICE_REALTIME_UPSTREAM="ws://127.0.0.1:${BACKEND_PORT}/v1/realtime" \
STARTUP_GREETING="" \
HF_S2S_UPSTREAM="$UPSTREAM" \
"$VENV/bin/uvicorn" --app-dir "$ROOT/hf-s2s" demo_proxy:app \
  --host 127.0.0.1 --port "$UI_PORT" &
children+=("$!")

echo "HF cascaded voice comparison -> http://localhost:${UI_PORT}/"

# Optional LAN endpoint for another device, behind the same certificate as the
# wss wrapper above. Requires accepting that certificate once.
if [[ -r "$SSL_CERT" && -r "$SSL_KEY" ]]; then
  SPEECH_TO_SPEECH_URL="/v1/realtime" \
  OPEN_VOICE_REALTIME_UPSTREAM="ws://127.0.0.1:${BACKEND_PORT}/v1/realtime" \
  STARTUP_GREETING="" \
  HF_S2S_UPSTREAM="$UPSTREAM" \
  "$VENV/bin/uvicorn" --app-dir "$ROOT/hf-s2s" demo_proxy:app \
    --host 0.0.0.0 --port "$LAN_UI_PORT" \
    --ssl-certfile "$SSL_CERT" \
    --ssl-keyfile "$SSL_KEY" &
  children+=("$!")

  echo "HF cascaded voice comparison (LAN) -> https://${LAN_IP}:${LAN_UI_PORT}/"
fi
if [[ "$PLATFORM" == "Darwin" ]]; then
  # macOS still ships Bash 3.2, which predates `wait -n`. Poll all children so
  # the supervisor retains the Linux behavior of stopping if any service exits.
  while true; do
    for pid in "${children[@]}"; do
      if ! kill -0 "$pid" 2>/dev/null; then
        wait "$pid"
        exit $?
      fi
    done
    sleep 1
  done
else
  wait -n "${children[@]}"
fi
