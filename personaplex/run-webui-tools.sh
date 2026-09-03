#!/usr/bin/env bash
# PersonaPlex + local Gemma E4B tool router + OpenClaw delegation.
set -euo pipefail
cd "$(dirname "$0")"
export PYTHONPATH="$PWD/src/moshi${PYTHONPATH:+:$PYTHONPATH}"
export HIP_VISIBLE_DEVICES=0
export PPLEX_OPENCLAW=1
export PPLEX_OPENCLAW_AGENT="${PPLEX_OPENCLAW_AGENT:-main}"
export PPLEX_ASR_MODEL="${PPLEX_ASR_MODEL:-small.en}"
export PPLEX_DIAG="${PPLEX_DIAG:-1}"
export PPLEX_ROUTER_MODEL="${PPLEX_ROUTER_MODEL:-gemma-e4b}"
export PPLEX_ROUTER_URL="${PPLEX_ROUTER_URL:-http://127.0.0.1:8011/v1/chat/completions}"
ROUTER_BIN="${PPLEX_ROUTER_BIN:-/home/nitayrabi/llama.cpp/build-hip/bin/llama-server}"
ROUTER_GGUF="${PPLEX_ROUTER_GGUF:-/home/nitayrabi/models/gemma-4-E4B_q4_0-it.gguf}"

"$ROUTER_BIN" \
  --model "$ROUTER_GGUF" --alias "$PPLEX_ROUTER_MODEL" \
  --host 127.0.0.1 --port 8011 --ctx-size 4096 --parallel 1 \
  --gpu-layers all --flash-attn on --reasoning off --no-webui &
router_pid=$!
cleanup() {
  kill "$router_pid" 2>/dev/null || true
  wait "$router_pid" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

for _ in $(seq 1 120); do
  if curl -fsS http://127.0.0.1:8011/health >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$router_pid" 2>/dev/null; then
    echo "Gemma E4B router exited during startup" >&2
    exit 1
  fi
  sleep 0.5
done
if ! curl -fsS http://127.0.0.1:8011/health >/dev/null 2>&1; then
  echo "Gemma E4B router did not become healthy" >&2
  exit 1
fi
SSL_DIR="${SSL_DIR:-$(cd "$(dirname "$0")" && pwd)/ssl}"; mkdir -p "$SSL_DIR"
echo "PersonaPlex + Gemma E4B + OpenClaw(agent=$PPLEX_OPENCLAW_AGENT) -> https://192.168.68.46:8998"
./.venv/bin/python -m moshi.server --ssl "$SSL_DIR" --port 8998 \
  --static "$PWD/src/client/dist" "$@"
