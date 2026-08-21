#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APP_ID="com.goshapps.GameHandler"
VERSION="$(python3 -c 'from gamehandler import __version__; print(__version__)')"
BRANCH="stable"

cd "$ROOT"
flatpak-builder \
  --user \
  --force-clean \
  --disable-rofiles-fuse \
  --install-deps-from=flathub \
  --default-branch="$BRANCH" \
  --repo="$ROOT/flatpak-repo" \
  "$ROOT/build-flatpak" \
  "$ROOT/build-aux/flatpak/$APP_ID.json"

mkdir -p "$ROOT/dist"
flatpak build-bundle \
  "$ROOT/flatpak-repo" \
  "$ROOT/dist/gamehandler-$VERSION.flatpak" \
  "$APP_ID" \
  "$BRANCH"

echo "Created $ROOT/dist/gamehandler-$VERSION.flatpak"