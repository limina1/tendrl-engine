#!/usr/bin/env bash
# Build the single-executable tendrl-engine bundle.
#
# Produces one binary (target/release/tendrl-engine) with:
#   - the SvelteKit SPA embedded (rust-embed, from web/build/)
#   - in-process ONNX embeddings (built in, no Python sidecar)
#
# The embedding model is NOT baked in; fastembed downloads it from HuggingFace
# on first use and caches it. Run the binary, then open the browser it launches.
#
# Usage:
#   scripts/build-bundle.sh                       # build at the current version
#   scripts/build-bundle.sh --bump patch|minor|major   # bump first, then build
#   scripts/build-bundle.sh --version X.Y.Z       # set an exact version, then build
set -euo pipefail
cd "$(dirname "$0")/.."

# Optional version bump before building. The release name below is derived from
# Cargo.toml, so bumping here stamps the new number onto this build's artifact;
# release-notes.sh then records the commits landed since the last release tag.
case "${1:-}" in
    --bump)    scripts/bump-version.sh "${2:?--bump needs major|minor|patch}"; scripts/release-notes.sh ;;
    --version) scripts/bump-version.sh "${2:?--version needs X.Y.Z}"; scripts/release-notes.sh ;;
    "")        ;;
    *) echo "Usage: $0 [--bump major|minor|patch | --version X.Y.Z]" >&2; exit 1 ;;
esac

echo "==> Building web frontend (pnpm)…"
pnpm -C web install --frozen-lockfile
# esbuild installs its native binary via a postinstall "build script". pnpm gates
# those behind approval (web/pnpm-workspace.yaml), but an already-populated
# node_modules won't re-run an approved script on reinstall — so force it. No-op
# once the binary is present; without it, `vite build` fails to load esbuild.
pnpm -C web rebuild esbuild
# Invoke vite directly rather than `pnpm run build`: the package script is just
# `vite build`, and `pnpm run` does a pre-run deps-status check that can spuriously
# re-trigger (and fail) install in CI/fresh environments. `exec` skips that.
pnpm -C web exec vite build

if [[ ! -f web/build/index.html ]]; then
    echo "ERROR: web/build/index.html missing after pnpm build." >&2
    exit 1
fi

echo "==> Building engine (cargo release)…"
# Touch web/build so build.rs' rerun-if-changed picks up the fresh SPA.
touch web/build
cargo build --release

# Stamp a versioned, release-named copy for handing to testers. The version is
# read from Cargo.toml so it can never drift from the actual build. Linux-only
# for now — no arch/triple in the name; keep it short.
version=$(grep -m1 '^version' Cargo.toml | sed -E 's/version *= *"([^"]+)".*/\1/')
mkdir -p dist
release_name="tendrl-engine-${version}"
cp target/release/tendrl-engine "dist/${release_name}"

echo ""
echo "Done: target/release/tendrl-engine"
echo "Release: dist/${release_name}  (hand this to testers)"
echo "Run it:  ./dist/${release_name}"
echo "(opens http://127.0.0.1:3030/ — log in with a NIP-07 browser extension)"
