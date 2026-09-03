#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
ENGINE_DIR="${VC_SRC:-$HOME/llama-voicechat.cpp}"
MODEL_DIR="${VC_MODEL_ROOT:-$HOME/models/voicechat}"
OUT_DIR="${VC_MODELS:-$MODEL_DIR/llamacpp}"
PYTHON_BIN="${VC_PYTHON:-$PROJECT_DIR/personaplex/.venv/bin/python}"
QUANT="${1:-Q8_0}"

case "$QUANT" in
    Q4_0|Q8_0|F16) ;;
    *) echo "usage: $0 [Q4_0|Q8_0|F16]" >&2; exit 2 ;;
esac

mkdir -p "$MODEL_DIR" "$OUT_DIR"
if [[ "$QUANT" == "F16" ]]; then
    SOURCE="nemotron_voicechat_11b-f16.gguf"
else
    SOURCE="nemotron_voicechat_11b-$QUANT.gguf"
fi
SOURCE_PATH="$MODEL_DIR/$SOURCE"
REF_DIR="$MODEL_DIR/ref_nano9b"

hub_download() {
    "$PYTHON_BIN" - "$1" "$2" "$3" <<'PY'
import sys
from huggingface_hub import hf_hub_download
hf_hub_download(repo_id=sys.argv[1], filename=sys.argv[2], local_dir=sys.argv[3])
PY
}

if [[ ! -f "$SOURCE_PATH" ]]; then
    hub_download hoidhxd/NVIDIA-NemotronLabs-VoiceChat-11B-GGUF "$SOURCE" "$MODEL_DIR"
fi
if [[ ! -f "$REF_DIR/tokenizer.json" ]]; then
    hub_download nvidia/NVIDIA-Nemotron-Nano-9B-v2 config.json "$REF_DIR"
    hub_download nvidia/NVIDIA-Nemotron-Nano-9B-v2 tokenizer.json "$REF_DIR"
    hub_download nvidia/NVIDIA-Nemotron-Nano-9B-v2 tokenizer_config.json "$REF_DIR"
fi

LLM="$OUT_DIR/nemotron_voicechat_11b-stt-llm-$QUANT.gguf"
MMPROJ="$OUT_DIR/mmproj-voicechat-perception-$QUANT.gguf"
TTS="$OUT_DIR/voicechat-tts-$QUANT.gguf"

if [[ ! -f "$LLM" ]]; then
    "$PYTHON_BIN" "$ENGINE_DIR/tools/voicechat/convert_voicechat_to_nemotron_h.py" \
        "$SOURCE_PATH" --ref-dir "$REF_DIR" -o "$LLM"
fi
if [[ ! -f "$MMPROJ" ]]; then
    "$PYTHON_BIN" "$ENGINE_DIR/tools/voicechat/convert_voicechat_perception_to_mmproj.py" \
        "$SOURCE_PATH" -o "$MMPROJ"
fi
if [[ ! -f "$TTS" ]]; then
    "$PYTHON_BIN" "$ENGINE_DIR/tools/voicechat/convert_voicechat_tts_to_gguf.py" \
        "$SOURCE_PATH" --ref-dir "$REF_DIR" -o "$TTS"
fi

echo "$QUANT assets are ready in $OUT_DIR"
