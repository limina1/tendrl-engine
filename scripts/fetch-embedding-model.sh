#!/usr/bin/env bash
# Pre-download the ONNX embedding model into fastembed's cache with a fast
# downloader (curl), so the engine loads it instantly.
#
# Why: fastembed's built-in downloader (hf-hub 0.4.3) crawls — often stalling —
# on HuggingFace's Xet-backed repos, while plain curl pulls the same 86 MB in a
# couple of seconds. fastembed checks its cache (file existence only, no network)
# before downloading, so seeding the cache skips the slow path entirely.
#
# Usage:
#   scripts/fetch-embedding-model.sh                  # default model + default cache
#   FASTEMBED_CACHE_DIR=/path scripts/fetch-embedding-model.sh
#   EMBED_MODEL_REPO=Xenova/all-MiniLM-L12-v2 scripts/fetch-embedding-model.sh
#
# The default cache path matches what the engine uses for the default data dir
# (<data_dir parent>/fastembed_cache). If you run the engine with a custom
# --data-dir or FASTEMBED_CACHE_DIR, pass the same FASTEMBED_CACHE_DIR here.
set -euo pipefail

REPO="${EMBED_MODEL_REPO:-Qdrant/all-MiniLM-L6-v2-onnx}"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
CACHE="${FASTEMBED_CACHE_DIR:-$DATA_HOME/nostr-engine/fastembed_cache}"

# Files fastembed needs (model + tokenizer). Skips README/.gitattributes.
FILES="config.json special_tokens_map.json tokenizer.json tokenizer_config.json vocab.txt model.onnx"

# hf-hub cache folder name: "Qdrant/all-MiniLM-L6-v2-onnx" -> "models--Qdrant--all-MiniLM-L6-v2-onnx"
FOLDER="models--${REPO//\//--}"

echo "Model:  $REPO"
echo "Cache:  $CACHE"

# Resolve the current commit sha (the offline cache check only needs refs/main to
# match the snapshots/<sha> dir name; the real sha keeps it canonical).
SHA="$(curl -fsSL "https://huggingface.co/api/models/$REPO" \
  | grep -oE '"sha"[[:space:]]*:[[:space:]]*"[0-9a-f]+"' | head -1 | grep -oE '[0-9a-f]{6,}')"
if [ -z "${SHA:-}" ]; then echo "ERROR: could not resolve commit sha for $REPO" >&2; exit 1; fi
echo "Commit: $SHA"

DEST="$CACHE/$FOLDER/snapshots/$SHA"
mkdir -p "$DEST" "$CACHE/$FOLDER/refs"

for f in $FILES; do
    printf '  fetching %s ... ' "$f"
    curl -fsSL -o "$DEST/$f" "https://huggingface.co/$REPO/resolve/main/$f"
    echo "ok ($(du -h "$DEST/$f" | cut -f1))"
done
printf '%s' "$SHA" > "$CACHE/$FOLDER/refs/main"

echo ""
echo "Seeded. Enable embeddings in config.toml:"
echo "  [embedding]"
echo "  enabled = true"
echo "Then restart the engine — it will load the model from cache (no download)."
