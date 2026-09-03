#!/usr/bin/env bash
# Hugging Face cascaded voice-agent comparison: Parakeet -> Gemma E4B -> Qwen3-TTS.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UPSTREAM="$ROOT/.tmp/speech-to-speech"
VENV="$UPSTREAM/.venv"
SSL_DIR="${SSL_DIR:-$ROOT/personaplex/ssl}"
LAN_IP="${HF_S2S_LAN_IP:-192.168.68.46}"
BACKEND_PORT="${HF_S2S_BACKEND_PORT:-8766}"
WSS_PORT="${HF_S2S_WSS_PORT:-8765}"
UI_PORT="${HF_S2S_UI_PORT:-9000}"
LAN_UI_PORT="${HF_S2S_LAN_UI_PORT:-9001}"
LLM_PORT="${HF_S2S_LLM_PORT:-8011}"
LLM_BIN="${HF_S2S_LLM_BIN:-/home/nitayrabi/llama.cpp/build-hip/bin/llama-server}"
LLM_MODEL="${HF_S2S_LLM_MODEL:-/home/nitayrabi/models/gemma-4-E4B_q4_0-it.gguf}"
TTS_MODEL="${HF_S2S_TTS_MODEL:-Qwen/Qwen3-TTS-12Hz-0.6B-CustomVoice}"
TTS_HIP_BIN="${HF_S2S_TTS_HIP_BIN:-$ROOT/.tmp/qwen3-tts-hip/target/release/tts-server}"
TTS_HIP_MODEL_DIR="${HF_S2S_TTS_HIP_MODEL_DIR:-/home/nitayrabi/.cache/huggingface/hub/models--Qwen--Qwen3-TTS-12Hz-0.6B-CustomVoice/snapshots/85e237c12c027371202489a0ec509ded67b5e4b5}"
TTS_HIP_PORT="${HF_S2S_TTS_HIP_PORT:-8021}"

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

"$LLM_BIN" \
  --model "$LLM_MODEL" --alias gemma-e4b \
  --host 127.0.0.1 --port "$LLM_PORT" --ctx-size 4096 --parallel 1 \
  --gpu-layers all --flash-attn on --reasoning off --no-webui &
children+=("$!")

for _ in $(seq 1 120); do
  if curl -fsS "http://127.0.0.1:${LLM_PORT}/health" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "${children[0]}" 2>/dev/null; then
    echo "Gemma E4B exited during startup" >&2
    exit 1
  fi
  sleep 0.5
done

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
  --stt parakeet-tdt \
  --parakeet_tdt_device cuda \
  --parakeet_tdt_compute_type float16 \
  --llm_backend chat-completions \
  --model_name gemma-e4b \
  --responses_api_base_url "http://127.0.0.1:${LLM_PORT}/v1" \
  --responses_api_api_key "" \
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
  --init_chat_prompt "You are a natural, concise voice assistant. Speak conversationally. Use an available tool whenever it is needed, then briefly tell the user the result." &
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

socat \
  "OPENSSL-LISTEN:${WSS_PORT},reuseaddr,fork,cert=${SSL_DIR}/cert.pem,key=${SSL_DIR}/key.pem,verify=0" \
  "TCP:127.0.0.1:${BACKEND_PORT}" &
children+=("$!")

# Same-machine endpoint: localhost is a browser secure context, so microphone
# access works without accepting a development certificate.
SPEECH_TO_SPEECH_URL="ws://localhost:${BACKEND_PORT}/v1/realtime" \
STARTUP_GREETING="" \
"$VENV/bin/uvicorn" --app-dir demo server:app \
  --host 0.0.0.0 --port "$UI_PORT" &
children+=("$!")

# Optional LAN endpoint for another device. It uses the existing development
# certificate and therefore requires accepting that certificate once.
SPEECH_TO_SPEECH_URL="wss://${LAN_IP}:${WSS_PORT}/v1/realtime" \
STARTUP_GREETING="" \
"$VENV/bin/uvicorn" --app-dir demo server:app \
  --host 0.0.0.0 --port "$LAN_UI_PORT" \
  --ssl-certfile "${SSL_DIR}/cert.pem" \
  --ssl-keyfile "${SSL_DIR}/key.pem" &
children+=("$!")

echo "HF cascaded voice comparison -> http://localhost:${UI_PORT}/"
echo "HF cascaded voice comparison (LAN) -> https://${LAN_IP}:${LAN_UI_PORT}/"
wait -n "${children[@]}"
