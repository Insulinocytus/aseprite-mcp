#!/usr/bin/env bash
# Refresh skills/aseprite-lua/references/ from a pinned aseprite/api commit.
# To upgrade: change COMMIT, run this script, commit the diff.
set -euo pipefail

COMMIT=662b12efe00fd374909bc3a0e9cce5d0d0d35d63
DEST="$(cd "$(dirname "$0")/.." && pwd)/skills/aseprite-lua/references"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

git init -q "$TMP"
git -C "$TMP" fetch -q --depth 1 https://github.com/aseprite/api.git "$COMMIT"
git -C "$TMP" checkout -q FETCH_HEAD

rm -rf "$DEST"
mkdir -p "$DEST"
cp -r "$TMP/api" "$TMP/Changes.md" "$TMP/LICENSE.txt" "$TMP/README.md" "$DEST/"
echo "$COMMIT" > "$DEST/COMMIT"
echo "references pinned to aseprite/api@$COMMIT"
