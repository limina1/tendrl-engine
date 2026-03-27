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

# Create persistent venv on first run
if [ ! -d .venv ]; then
    echo "Creating sidecar venv..."
    uv venv
    uv pip install sentence-transformers flask pymupdf python-docx ebooklib lxml
fi

exec .venv/bin/python embed.py "$@"
