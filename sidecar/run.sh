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

exec uv run --with sentence-transformers --with flask --with pymupdf --with python-docx --with ebooklib --with lxml python embed.py "$@"
