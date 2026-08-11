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

# --- Guards against accidental bumps (shell-history replays, habitual --bump) ---

# 1. Refuse to stack a bump on an unreleased one: if the working tree's version
#    already differs from HEAD's, a previous bump is sitting uncommitted — a
#    second bump would silently skip version numbers (0.1.0->0.3.0 happened).
head_version=$(git show HEAD:Cargo.toml 2>/dev/null | grep -m1 '^version' | sed -E 's/version *= *"([^"]+)".*/\1/' || true)
if [[ -n "$head_version" && "$head_version" != "$current" ]]; then
  echo "ERROR: Cargo.toml already carries an uncommitted bump (HEAD is ${head_version}, tree is ${current})." >&2
  echo "       Commit/release that first, or revert it:  git checkout -- Cargo.toml Cargo.lock" >&2
  exit 1
fi

# 2. Refuse when there is nothing to release: no commits since the last v* tag
#    means a rebuild is wanted, not a new version — run the build without --bump.
last_tag=$(git tag --list 'v*' --sort=-version:refname | head -1)
if [[ -n "$last_tag" ]] && git rev-parse -q --verify "$last_tag" >/dev/null \
   && [[ -z "$(git log --oneline "${last_tag}..HEAD" | head -1)" ]]; then
  echo "ERROR: no commits since ${last_tag} — nothing to release." >&2
  echo "       Rebuilding the same version? Run the build script without --bump." >&2
  exit 1
fi

# 3. Interactive confirmation, so a stray up-arrow re-run can't bump on its own.
if [[ -t 0 ]]; then
  read -r -p "==> bump version ${current} -> ${new}? [y/N] " reply
  [[ "$reply" =~ ^[Yy] ]] || { echo "aborted — version stays ${current}"; exit 1; }
fi

# Rewrite only the first (package) version line. GNU sed: 0,/re/ bounds the range
# to the first match; s//…/ reuses that same regex as the substitution pattern.
sed -i -E "0,/^version *= *\"[^\"]+\"/s//version = \"${new}\"/" "$manifest"

# Keep the Android host on the same version train: the engine runs in-process
# inside the APK, so the APK's versionName (tauri.conf.json "version") and the
# mobile crate mirror the engine version. tauri derives the APK versionCode as
# major*1000000 + minor*1000 + patch — monotonic while versions only go up.
sed -i -E "0,/\"version\": *\"[^\"]+\"/s//\"version\": \"${new}\"/" mobile/src-tauri/tauri.conf.json
sed -i -E "0,/^version *= *\"[^\"]+\"/s//version = \"${new}\"/" mobile/src-tauri/Cargo.toml

# Keep Cargo.lock's own package entry in step so the next build doesn't re-touch
# it as a side effect. Harmless if cargo is absent — the build resolves it anyway.
if command -v cargo >/dev/null 2>&1; then
  cargo update -p nostr-engine >/dev/null 2>&1 || true
  (cd mobile/src-tauri && cargo update -p tendrl-mobile >/dev/null 2>&1) || true
fi

echo "==> version: ${current} -> ${new}"
echo "    next:  build (the artifact is now stamped ${new}), then  git tag v${new}"
