#!/usr/bin/env bash
# Bump the tendrl-engine release version in Cargo.toml (and sync Cargo.lock).
#
# Cargo.toml's [package] version is the single source of truth for a release
# number: build-bundle.sh / build-portable.sh derive the artifact name from it,
# so the binary can never drift from the version. Bump it here (or via the
# build scripts' --bump flag) before cutting a release, then tag v<version>.
#
# Usage:
#   scripts/bump-version.sh patch     # 0.1.0 -> 0.1.1   (also: minor, major)
#   scripts/bump-version.sh minor     # 0.1.3 -> 0.2.0   (resets patch)
#   scripts/bump-version.sh major     # 0.2.5 -> 1.0.0   (resets minor+patch)
#   scripts/bump-version.sh 1.4.2     # set an exact version
#   scripts/bump-version.sh           # print current version, change nothing
set -euo pipefail
cd "$(dirname "$0")/.."

manifest="Cargo.toml"
# The [package] version is the first top-of-line `version =`; dependency versions
# are all indented, so `^version` matches only the package's own line.
current=$(grep -m1 '^version' "$manifest" | sed -E 's/version *= *"([^"]+)".*/\1/')

if [[ $# -eq 0 ]]; then
  echo "$current"
  exit 0
fi

# Split semver into parts (drop any -pre / +build metadata for the arithmetic).
IFS='.' read -r major minor patch <<<"${current%%[-+]*}"

case "$1" in
  major) new="$((major + 1)).0.0" ;;
  minor) new="${major}.$((minor + 1)).0" ;;
  patch) new="${major}.${minor}.$((patch + 1))" ;;
  [0-9]*.[0-9]*.[0-9]*) new="$1" ;;
  *)
    echo "ERROR: expected 'major', 'minor', 'patch', or an explicit X.Y.Z — got '$1'" >&2
    exit 1
    ;;
esac

if [[ "$new" == "$current" ]]; then
  echo "==> version unchanged ($current)"
  exit 0
fi

# Rewrite only the first (package) version line. GNU sed: 0,/re/ bounds the range
# to the first match; s//…/ reuses that same regex as the substitution pattern.
sed -i -E "0,/^version *= *\"[^\"]+\"/s//version = \"${new}\"/" "$manifest"

# Keep Cargo.lock's own package entry in step so the next build doesn't re-touch
# it as a side effect. Harmless if cargo is absent — the build resolves it anyway.
if command -v cargo >/dev/null 2>&1; then
  cargo update -p nostr-engine >/dev/null 2>&1 || true
fi

echo "==> version: ${current} -> ${new}"
echo "    next:  build (the artifact is now stamped ${new}), then  git tag v${new}"
