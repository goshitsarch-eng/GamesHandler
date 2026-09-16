#!/usr/bin/env bash
#
# release-check.sh — the tag must say the same version everything else says.
#
#   scripts/release-check.sh v0.8.0
#
# Checks, in order, and fails loudly on the first disagreement:
#
#   1. the tag is `vX.Y.Z` (optionally -rc.N / -beta.N style suffix rejected
#      only if the workspace version does not carry the same suffix)
#   2. workspace version in the root Cargo.toml
#   3. the newest <release version> in the AppStream metainfo
#   4. gamehandler/__init__.py __version__ (the Python reference tree)
#
# A release whose four sources disagree ships a binary that reports one
# version under a tag that claims another — that mismatch is cheap to stop
# here and expensive to explain later.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

[ $# -eq 1 ] || { echo "usage: scripts/release-check.sh TAG   (e.g. v0.8.0)" >&2; exit 2; }
TAG="$1"

if ! [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "release-check.sh: tag '$TAG' is not shaped like vX.Y.Z (optional -suffix)" >&2
    exit 1
fi
TAG_VERSION="${TAG#v}"

version_of() {
  awk '
    /^\[package\]/           { section = "package";   next }
    /^\[workspace\.package\]/ { section = "workspace"; next }
    /^\[/                    { section = "";          next }
    section && /^version[ \t]*=[ \t]*"/ {
      sub(/^version[ \t]*=[ \t]*"/, ""); sub(/".*$/, ""); print; exit
    }
  ' "$1"
}

CARGO_VERSION="$(version_of "$ROOT/crates/app/Cargo.toml" 2>/dev/null || true)"
[ -n "$CARGO_VERSION" ] || CARGO_VERSION="$(version_of "$ROOT/Cargo.toml")"
[ -n "$CARGO_VERSION" ] || { echo "release-check.sh: no workspace version in Cargo.toml" >&2; exit 1; }

PY_VERSION="$(awk -F'"' '/^__version__ = "/ {print $2; exit}' "$ROOT/gamehandler/__init__.py")"
META_VERSION="$(python3 - "$ROOT/data/com.goshapps.GameHandler.metainfo.xml" <<'PY'
import sys, xml.etree.ElementTree as ET
root = ET.parse(sys.argv[1]).getroot()
rel = root.find("./releases/release")
print(rel.get("version") if rel is not None else "")
PY
)"

fail=0
check() {
    if [ "$2" = "$TAG_VERSION" ]; then
        echo "ok   $1: $2"
    else
        echo "FAIL $1: '$2' != tag '$TAG_VERSION'" >&2
        fail=1
    fi
}

check "Cargo.toml workspace version" "$CARGO_VERSION"
check "metainfo newest <release>"    "$META_VERSION"
check "gamehandler/__init__.py"      "$PY_VERSION"

[ "$fail" -eq 0 ] || {
    echo "release-check.sh: tag $TAG disagrees with the tree — refusing to release a mismatch" >&2
    exit 1
}
echo "release-check.sh: all version sources agree on $TAG_VERSION"
