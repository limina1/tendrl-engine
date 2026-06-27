#!/usr/bin/env bash
# Build a PORTABLE single-executable tendrl-engine for distribution.
#
# `scripts/build-bundle.sh` links against the build host's glibc/OpenSSL. On a
# bleeding-edge distro (Arch, glibc 2.43) the result won't start on normal
# systems ("version `GLIBC_2.43' not found"). This script instead compiles the
# engine inside an OLD-glibc container (manylinux_2_28 = AlmaLinux 8, glibc
# 2.28) with a real gcc, so ONE binary runs on RHEL 8/9, Debian, Ubuntu LTS,
# Arch, etc.
#
# Why a container and not cargo-zigbuild: usearch's `numkong` SIMD kernels use
# AVX-512 intrinsics that zig's bundled clang rejects (the evex512 split). Real
# gcc compiles them fine, so we build natively against an old glibc instead.
#
# What makes the output portable:
#   - glibc 2.28 floor (forward-compatible: runs on every newer glibc too)
#   - TLS is rustls (pure Rust) everywhere incl. fastembed's HF downloader —
#     no libssl/libcrypto dependency at all
#   - onnxruntime is statically linked in (no libonnxruntime.so to ship)
# The embedding MODEL is still downloaded from HuggingFace on first run.
#
# Prereqs:  docker, pnpm/node (host, for the SPA)
# Usage:
#   scripts/build-portable.sh                          # build at the current version
#   scripts/build-portable.sh --bump patch|minor|major # bump first, then build
#   scripts/build-portable.sh --version X.Y.Z          # set an exact version, then build
set -euo pipefail
cd "$(dirname "$0")/.."

# Optional version bump before building (Cargo.toml is the single source of the
# release number; the tarball/binary are stamped from it downstream). The bump is
# followed by a CHANGELOG entry covering the commits since the last release tag.
case "${1:-}" in
    --bump)    scripts/bump-version.sh "${2:?--bump needs major|minor|patch}"; scripts/release-notes.sh ;;
    --version) scripts/bump-version.sh "${2:?--version needs X.Y.Z}"; scripts/release-notes.sh ;;
    "")        ;;
    *) echo "Usage: $0 [--bump major|minor|patch | --version X.Y.Z]" >&2; exit 1 ;;
esac

IMAGE="quay.io/pypa/manylinux_2_28_x86_64"
TARGET_SUBDIR="target/portable"
OUT="${TARGET_SUBDIR}/release/tendrl-engine"

command -v docker >/dev/null || { echo "ERROR: docker not found" >&2; exit 1; }
command -v pnpm   >/dev/null || { echo "ERROR: pnpm not found (needed to build the SPA)" >&2; exit 1; }

echo "==> Building web frontend on host (rust-embed bakes web/build/ into the binary)…"
pnpm -C web install --frozen-lockfile
pnpm -C web rebuild esbuild
pnpm -C web exec vite build
[[ -f web/build/index.html ]] || { echo "ERROR: web/build/index.html missing after build." >&2; exit 1; }
touch web/build   # let build.rs' rerun-if-changed pick up the fresh SPA

echo "==> Compiling engine in ${IMAGE} (glibc 2.28 floor)…"
# A named volume caches the container's cargo registry + rustup across runs and
# keeps them out of the host's ~/.cargo. CARGO_TARGET_DIR lands the output on the
# mounted source tree; we chown it back to the host user at the end.
docker run --rm -t \
  -v "$PWD":/src -w /src \
  -v tendrl-portable-cargo:/root/.cargo \
  -e CARGO_TARGET_DIR="/src/${TARGET_SUBDIR}" \
  -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  "$IMAGE" bash -euo pipefail -c '
    source /opt/rh/gcc-toolset-*/enable 2>/dev/null || true   # modern gcc for numkong AVX-512
    export CC=gcc CXX=g++
    if ! command -v cargo >/dev/null; then
      curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    fi
    source "$HOME/.cargo/env"
    # onnxruntime (prebuilt via fastembed ort-download-binaries) is compiled
    # against a newer libstdc++ whose headers reference __libc_single_threaded,
    # a symbol that only exists in glibc >= 2.32. Linking here against glibc 2.28
    # leaves it undefined. Supply a weak fallback (0 = "not single-threaded",
    # forcing the always-correct atomic paths); on a newer-glibc host libc`s
    # strong symbol takes precedence. Keeps the 2.28 portability floor intact.
    printf "char __libc_single_threaded __attribute__((weak)) = 0;\n" > /tmp/glibc_compat.c
    gcc -c -O2 -fPIC /tmp/glibc_compat.c -o /tmp/glibc_compat.o
    export RUSTFLAGS="${RUSTFLAGS:-} -Clink-arg=/tmp/glibc_compat.o"
    cargo build --release
    chown -R "${HOST_UID}:${HOST_GID}" "/src/'"${TARGET_SUBDIR}"'"
  '

echo ""
echo "==> Portability report for ${OUT}:"
echo "--- size ---"; ls -lh "${OUT}" | awk '{print "    "$5}'
echo "--- NEEDED shared libs ---"; objdump -p "${OUT}" | awk '/NEEDED/ {print "    "$2}'
echo -n "--- GLIBC   max required: "; objdump -T "${OUT}" | grep -oP 'GLIBC_\d+\.\d+'  | sort -V | tail -1
echo -n "--- GLIBCXX max required: "; objdump -T "${OUT}" | grep -oP 'GLIBCXX_\d+\.\d+' | sort -V | tail -1 || echo "(none)"
echo ""
# Ship the embedding model alongside the binary so end users get embeddings with
# no first-run HuggingFace download. `--fetch-model` downloads it into a `models/`
# folder next to the executable (the engine auto-detects that folder at runtime).
# The weights are platform-agnostic ONNX, so fetching with the just-built binary
# on this host is fine. Needs network here (the build host), not on the user's.
OUT_DIR="$(dirname "${OUT}")"
echo "==> Fetching embedding model beside the binary…"
"${OUT}" --fetch-model

echo "==> Packaging distributable tarball (binary + models/)…"
# Version-stamp the tarball name from Cargo.toml so the release filename can't
# drift from the build. Tag the matching commit `v<version>` when you ship.
version=$(grep -m1 '^version' Cargo.toml | sed -E 's/version *= *"([^"]+)".*/\1/')
TARBALL="${TARGET_SUBDIR}/tendrl-engine-${version}.tar.gz"
tar -C "${OUT_DIR}" -czf "${TARBALL}" tendrl-engine models

echo ""
echo "Done."
echo "  Single binary : ${OUT}"
echo "  Model folder  : ${OUT_DIR}/models  (ships beside the binary)"
echo "  Distributable : ${TARBALL}  ← share this"
echo ""
echo "Testers: extract it, then  ./tendrl-engine   (opens http://127.0.0.1:3030/)."
echo "The 'models' folder must stay next to the binary; no model download needed."
