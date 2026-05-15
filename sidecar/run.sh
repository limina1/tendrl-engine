#!/usr/bin/env bash
# Start the embedding sidecar
# Requires: uv (https://docs.astral.sh/uv/)
#
# Usage:
#   ./run.sh                              # default model
#   ./run.sh --model nomic-embed-text-v1.5  # custom model
#   ./run.sh --port 3032                  # custom port

set -e
cd "$(dirname "$0")"

# Create persistent venv on first run.
# Install from the pinned lockfile when present (reproducible); the
# unpinned requirements.txt is only a fallback. Regenerate the lock with
# `make lock-sidecar` / `make update-sidecar` from the repo root.
if [ ! -d .venv ]; then
    echo "Creating sidecar venv..."
    uv venv
    if [ -f requirements.lock ]; then
        uv pip sync requirements.lock
    else
        uv pip install -r requirements.txt
    fi
fi

# Use cached model, don't check HF Hub on startup
export HF_HUB_OFFLINE=1

exec .venv/bin/python embed.py "$@"
