#!/usr/bin/env bash
# Install the managed speech-to-speech Python runtime for Apple Silicon.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UPSTREAM="$ROOT/.tmp/speech-to-speech"
UPSTREAM_URL="${HF_S2S_REPOSITORY:-https://github.com/huggingface/speech-to-speech.git}"
# Known to contain the macOS Parakeet + Qwen3-TTS MLX path used by the launcher.
UPSTREAM_REF="${HF_S2S_REF:-e34312cf47cd0159ee82f0d34b02e72353b7752e}"

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "This setup script supports Apple Silicon macOS only." >&2
  exit 1
fi

for command_name in git uv; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Missing '${command_name}'. Install prerequisites with: brew install uv llama.cpp" >&2
    exit 1
  fi
done

if [[ ! -d "$UPSTREAM/.git" ]]; then
  if [[ -e "$UPSTREAM" ]]; then
    echo "${UPSTREAM} exists but is not a git checkout; move it aside and rerun." >&2
    exit 1
  fi
  mkdir -p "$(dirname "$UPSTREAM")"
  git clone "$UPSTREAM_URL" "$UPSTREAM"
  git -C "$UPSTREAM" checkout --detach "$UPSTREAM_REF"
else
  requested_commit="$(git -C "$UPSTREAM" rev-parse "${UPSTREAM_REF}^{commit}" 2>/dev/null || true)"
  current_commit="$(git -C "$UPSTREAM" rev-parse HEAD)"
  if [[ -z "$requested_commit" || "$current_commit" != "$requested_commit" ]]; then
    echo "Existing speech-to-speech checkout is at ${current_commit}, not ${UPSTREAM_REF}." >&2
    echo "Move ${UPSTREAM} aside and rerun, or set HF_S2S_REF=${current_commit} if that revision has the required macOS support." >&2
    exit 1
  fi
  echo "Using existing speech-to-speech checkout at ${current_commit}."
fi

uv sync --project "$UPSTREAM" --python 3.12

if [[ ! -x "$UPSTREAM/.venv/bin/speech-to-speech" ]]; then
  echo "Runtime installation finished without creating the expected CLI." >&2
  exit 1
fi

echo "Apple Silicon speech runtime is ready."
echo "Install llama.cpp if needed: brew install llama.cpp"
echo "The first voice-engine start will download the Parakeet and Qwen3-TTS MLX models."
