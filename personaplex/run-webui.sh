#!/usr/bin/env bash
# PersonaPlex-7B full-duplex voice web UI, pinned to the R9700 (gfx1201).
# Serves HTTPS on :8998 across all interfaces. Browser mic needs a secure
# context, hence --ssl (certs auto-generated via mkcert on first run).
set -euo pipefail
cd "$(dirname "$0")"

export HIP_VISIBLE_DEVICES=0          # R9700 only; keep the 890M iGPU out of it

SSL_DIR="${SSL_DIR:-$(cd "$(dirname "$0")" && pwd)/ssl}"
mkdir -p "$SSL_DIR"

echo "PersonaPlex -> https://localhost:8998"
echo "  from another device on the LAN:  https://192.168.68.46:8998"
echo "  over Tailscale:                  https://100.123.240.76:8998"
echo

exec ./.venv/bin/python -m moshi.server --ssl "$SSL_DIR" --port 8998 "$@"
