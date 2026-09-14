#!/usr/bin/env bash
# Supported lightweight Hugging Face cascade.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
exec "$ROOT/hf-s2s/run-comparison.sh" "$@"
