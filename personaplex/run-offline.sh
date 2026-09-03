#!/usr/bin/env bash
# Offline check: streams a WAV through PersonaPlex and writes a spoken reply.
# Needs no microphone -- use this to confirm the model works on this box.
set -euo pipefail
cd "$(dirname "$0")"
export HIP_VISIBLE_DEVICES=0

VOICE="${VOICE:-NATF2.pt}"
IN="${IN:-src/assets/test/input_assistant.wav}"
OUT="${OUT:-$HOME/personaplex/out/reply.wav}"
mkdir -p "$(dirname "$OUT")"

exec ./.venv/bin/python -m moshi.offline \
  --voice-prompt "$VOICE" \
  --input-wav "$IN" \
  --seed 42424242 \
  --output-wav "$OUT" \
  --output-text "${OUT%.wav}.json" "$@"
