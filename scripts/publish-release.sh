#!/usr/bin/env bash
# Publish a tagged version as a GitHub release: push the v<version> tag and
# create the release with the matching CHANGELOG.md section as its notes and
# the portable tarball (from scripts/build-portable.sh) as its asset.
#
# Fits the existing release workflow AFTER the build + release commit:
#   scripts/build-portable.sh --bump patch   # bump + changelog + tarball
#   git commit …                             # commit the release state
#   scripts/publish-release.sh               # tag (if needed), push tag, GitHub release
#
# The version is read from Cargo.toml (the single source of the release
# number). Passing an explicit older version backfills a notes-only release
# for an already-existing tag — older tarballs usually no longer exist, so
# backfills default to --no-asset semantics only if you pass the flag.
#
# Usage:
#   scripts/publish-release.sh               # current Cargo.toml version, tarball attached
#   scripts/publish-release.sh --no-asset    # notes-only release (no tarball)
#   scripts/publish-release.sh 0.8.1 --no-asset   # backfill an old tag, notes-only
#   scripts/publish-release.sh --dry-run     # show what would happen, change nothing
set -euo pipefail
cd "$(dirname "$0")/.."

version=""
no_asset=false
dry_run=false
for a in "$@"; do
  case "$a" in
    --no-asset) no_asset=true ;;
    --dry-run)  dry_run=true ;;
    -*) echo "ERROR: unknown flag '$a' (use --no-asset, --dry-run, or pass a version)" >&2; exit 1 ;;
    *)  [[ "$a" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]] || { echo "ERROR: positional arg must be a version like 0.8.1 (assets are located from it, not passed as paths) — got '$a'" >&2; exit 1; }
        version="$a" ;;
  esac
done

current=$(grep -m1 '^version' Cargo.toml | sed -E 's/version *= *"([^"]+)".*/\1/')
[[ -n "$version" ]] || version="$current"
tag="v${version}"

command -v gh >/dev/null || { echo "ERROR: gh (GitHub CLI) not found" >&2; exit 1; }
gh auth status >/dev/null 2>&1 || { echo "ERROR: gh is not authenticated (run: gh auth login)" >&2; exit 1; }

# Release notes = the CHANGELOG.md section for exactly this version (written by
# scripts/release-notes.sh at bump time). Exact-prefix match, no regex escaping.
notes=$(awk -v head="## v${version} " '
  substr($0, 1, length(head)) == head { found = 1; next }
  found && /^## v/ { exit }
  found { print }
' CHANGELOG.md)
notes=$(printf '%s' "$notes" | sed -e '/./,$!d')   # drop leading blank lines
[[ -n "$notes" ]] || { echo "ERROR: no '## v${version}' section in CHANGELOG.md — run scripts/release-notes.sh first" >&2; exit 1; }

# Tag: the current version may not be tagged yet (tagging is part of shipping —
# create it at HEAD). An explicit older version must already have its tag; we
# won't guess which historical commit it belongs to.
create_tag=false
if ! git rev-parse -q --verify "refs/tags/${tag}" >/dev/null; then
  if [[ "$version" == "$current" ]]; then
    create_tag=true
  else
    echo "ERROR: tag ${tag} does not exist and ${version} is not the current Cargo.toml version — cannot infer its commit" >&2
    exit 1
  fi
fi

# Assets: the version-stamped tarball from build-portable.sh (required), plus
# the signed Android APK from build-android.sh when one was built for this
# version (optional — desktop-only releases are fine).
tarball="target/portable/tendrl-engine-${version}.tar.gz"
if ! $no_asset && [[ ! -f "$tarball" ]]; then
  echo "ERROR: ${tarball} not found — run scripts/build-portable.sh, or pass --no-asset for a notes-only release" >&2
  exit 1
fi
apk="target/android/tendrl-${version}-arm64.apk"
if ! $no_asset && [[ ! -f "$apk" ]]; then
  echo "NOTE: no ${apk} — release ships without an Android APK (scripts/build-android.sh to add one)."
  apk=""
fi
$no_asset && apk=""

if gh release view "$tag" >/dev/null 2>&1; then
  echo "ERROR: release ${tag} already exists on GitHub (use 'gh release upload ${tag} <file> --clobber' to replace assets)" >&2
  exit 1
fi

if $dry_run; then
  echo "==> DRY RUN — would publish ${tag}:"
  $create_tag && echo "    tag:    create ${tag} at HEAD ($(git rev-parse --short HEAD)), then push" \
              || echo "    tag:    push existing ${tag} ($(git rev-parse --short "${tag}^{commit}"))"
  $no_asset   && echo "    asset:  none (--no-asset)" \
              || echo "    asset:  ${tarball} ($(ls -lh "$tarball" | awk '{print $5}'))"
  [[ -n "$apk" ]] && echo "    asset:  ${apk} ($(ls -lh "$apk" | awk '{print $5}'))"
  echo "    notes:"
  printf '%s\n' "$notes" | sed 's/^/      /'
  exit 0
fi

$create_tag && { git tag "$tag"; echo "==> Tagged HEAD as ${tag}"; }

echo "==> Pushing ${tag} to origin…"
git push origin "refs/tags/${tag}"

echo "==> Creating GitHub release ${tag}…"
args=("$tag" --title "tendrl-engine ${tag}" --notes-file -)
$no_asset || args+=("$tarball")
[[ -n "$apk" ]] && args+=("$apk")
printf '%s\n' "$notes" | gh release create "${args[@]}"

# The tag makes its commits reachable, but if no pushed branch contains it the
# repo's branch view still looks behind — remind rather than push silently.
if [[ -z "$(git branch -r --contains "$tag" 2>/dev/null)" ]]; then
  echo "NOTE: no remote branch contains ${tag} yet — push your branch too (e.g. git push origin master)."
fi
