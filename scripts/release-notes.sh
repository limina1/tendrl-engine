#!/usr/bin/env bash
# Generate a CHANGELOG section from the git history since the last release.
#
# "Since the last release" = commits after the most recent v<X.Y.Z> tag (the tag
# the release workflow creates per version). With no such tag yet — i.e. before
# the first tagged release — it falls back to the full history, giving a complete
# first changelog. Merge commits are dropped; one bullet per real commit subject.
#
# The build scripts call this right after --bump, so a release build stamps the
# new version AND records what landed since the previous one. Run standalone too.
#
# Usage:
#   scripts/release-notes.sh            # section for the current Cargo.toml version
#   scripts/release-notes.sh 1.2.0      # section for an explicit version
#   scripts/release-notes.sh --print    # print to stdout, leave CHANGELOG.md alone
set -euo pipefail
cd "$(dirname "$0")/.."

print_only=false
version=""
for a in "$@"; do
  case "$a" in
    --print) print_only=true ;;
    -*) echo "ERROR: unknown flag '$a' (use --print, or pass a version)" >&2; exit 1 ;;
    *) version="$a" ;;
  esac
done
[[ -n "$version" ]] || version=$(grep -m1 '^version' Cargo.toml | sed -E 's/version *= *"([^"]+)".*/\1/')

# Anchor on the newest v* release tag; before the first one, take all history.
anchor=$(git tag --list 'v*' --sort=-version:refname | head -1)
if [[ -n "$anchor" ]]; then
  range="${anchor}..HEAD"
else
  range="HEAD"
fi

commits=$(git log --no-merges --format='- %s' "$range")
[[ -n "$commits" ]] || commits="- (no new commits since ${anchor:-the start})"

date=$(git log -1 --format=%cd --date=short)
entry="## v${version} — ${date}

${commits}
"

if $print_only; then
  printf '%s\n' "$entry"
  exit 0
fi

# Prepend the new section so the newest release sits at the top of the file.
touch CHANGELOG.md
{ printf '%s\n' "$entry"; cat CHANGELOG.md; } >CHANGELOG.md.tmp && mv CHANGELOG.md.tmp CHANGELOG.md
echo "==> CHANGELOG.md: prepended v${version} (range: ${range})"
