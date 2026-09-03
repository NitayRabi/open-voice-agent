#!/usr/bin/env bash
# VoiceChat 11B + OpenClaw, hands-free UI on https://<host>:8999
set -euo pipefail
cd "$(dirname "$0")/voicechat"
exec ../personaplex/.venv/bin/python vc_openclaw.py "$@"
