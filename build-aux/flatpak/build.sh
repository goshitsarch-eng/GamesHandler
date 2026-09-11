#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APP_ID="com.goshapps.GameHandler"
BRANCH="stable"

# The version comes from the Cargo workspace, not from the retired Python tree.
# Member crates inherit `version` from `[workspace.package]` in the root
# Cargo.toml, so a member that sets its own `version = "…"` in `[package]`
# wins and the workspace value is the fallback. Parsed with awk rather than
# `cargo metadata`/jq so the script needs no JSON tooling on the host.
version_of() {
  awk '
    /^\[package\]/         { section = "package";   next }
    /^\[workspace\.package\]/ { section = "workspace"; next }
    /^\[/                  { section = "";          next }
    section && /^version[ \t]*=[ \t]*"/ {
      sub(/^version[ \t]*=[ \t]*"/, ""); sub(/".*$/, ""); print; exit
    }
  ' "$1"
}

VERSION="$(version_of "$ROOT/crates/app/Cargo.toml" 2>/dev/null || true)"
if [ -z "$VERSION" ]; then
  VERSION="$(version_of "$ROOT/Cargo.toml")"
fi
if [ -z "$VERSION" ]; then
  echo "build.sh: could not determine the version from the Cargo workspace" >&2
  exit 1
fi

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
