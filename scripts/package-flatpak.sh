#!/usr/bin/env bash
#
# package-flatpak.sh — build the Flatpak bundle for one architecture.
#
#   scripts/package-flatpak.sh --arch x86_64
#   scripts/package-flatpak.sh --arch aarch64
#
# The checked-in manifest (build-aux/flatpak/com.goshapps.GameHandler.json) is
# the x86_64 build: it uses the org.winehq.Wine base app, the i386 compat
# extension, --allow=multiarch, and ships DXVK's x86 DLLs. Flathub publishes no
# aarch64 org.winehq.Wine (verified 2026-10: `flatpak remote-info flathub
# runtime/org.winehq.Wine/aarch64/stable-25.08` finds no ref), so for aarch64
# this script generates a variant manifest with the Wine base, the x86-only
# extensions, the multiarch finish-arg and the dxvk-runtime module removed.
# The gamehandler module itself — the application — is architecture-neutral
# Rust and builds identically on both.
#
# The bundle is verified before it is named a release artifact: the ref that
# went into it must exist in the local build repo under the requested arch.
#
#   GAMEHANDLER_BUILD_OFFLINE=1   same flag build.sh honours: no network at
#                                 all (needs runtimes already installed)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ID="com.goshapps.GameHandler"
BRANCH="stable"

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

usage() {
    cat <<'EOF'
usage: scripts/package-flatpak.sh --arch ARCH [--version VERSION] [--out DIR]

  --arch      x86_64 or aarch64
  --version   release version (default: the workspace version in Cargo.toml)
  --out       output directory for the .flatpak bundle (default: dist/)

Environment: GAMEHANDLER_BUILD_OFFLINE=1 disables all network use.
EOF
}

ARCH="" VERSION="" OUT="$ROOT/dist"
while [ $# -gt 0 ]; do
    case "$1" in
        --arch)    ARCH="${2:?--arch needs a value}"; shift 2 ;;
        --version) VERSION="${2:?--version needs a value}"; shift 2 ;;
        --out)     OUT="${2:?--out needs a value}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "package-flatpak.sh: unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done
case "$ARCH" in
    x86_64|aarch64) ;;
    *) echo "package-flatpak.sh: --arch must be x86_64 or aarch64 (got: '$ARCH')" >&2; exit 2 ;;
esac

if [ -z "$VERSION" ]; then
    VERSION="$(version_of "$ROOT/crates/app/Cargo.toml" 2>/dev/null || true)"
    [ -n "$VERSION" ] || VERSION="$(version_of "$ROOT/Cargo.toml")"
fi
[ -n "$VERSION" ] || { echo "package-flatpak.sh: could not determine the version" >&2; exit 1; }

SRC_MANIFEST="$ROOT/build-aux/flatpak/$APP_ID.json"
MANIFEST="$SRC_MANIFEST"
cd "$ROOT"

if [ "$ARCH" = "aarch64" ]; then
    # The Wine base app and the 32-bit compat/GL extensions exist on
    # flathub only for x86_64; DXVK's bundled tarballs are x86 Windows
    # DLLs that cannot run under an aarch64 Wine. The variant drops all
    # four rather than shipping entries that cannot resolve.
    #
    # The variant is written BESIDE the source manifest, not under target/:
    # the manifest's `dir` source (`path: "../.."`) and its bare
    # `cargo-sources.json` string source are both resolved relative to the
    # manifest's own directory, so generating it anywhere else needs those
    # paths rewritten — a copy of the source-layout detail that would drift.
    # Same directory means no rewriting at all.
    MANIFEST="$ROOT/build-aux/flatpak/$APP_ID.$ARCH.json"
    python3 - "$SRC_MANIFEST" "$MANIFEST" <<'PY'
import json, sys
src, dest = sys.argv[1], sys.argv[2]
m = json.load(open(src))
for key in ("base", "base-version", "inherit-extensions"):
    m.pop(key, None)
m["finish-args"] = [a for a in m["finish-args"] if a != "--allow=multiarch"]
m["modules"] = [mod for mod in m["modules"] if mod.get("name") != "dxvk-runtime"]
json.dump(m, open(dest, "w"), indent=2)
PY
    echo "package-flatpak.sh: generated aarch64 manifest (no Wine base, no i386 compat, no DXVK)"
fi

FLAGS=(
  --user
  --force-clean
  --disable-rofiles-fuse
  --arch="$ARCH"
  --default-branch="$BRANCH"
  --repo="$ROOT/flatpak-repo"
)
if [ "${GAMEHANDLER_BUILD_OFFLINE:-0}" = "1" ]; then
  FLAGS+=(--disable-download)
else
  FLAGS+=(--install-deps-from=flathub)
fi

flatpak-builder \
  "${FLAGS[@]}" \
  "$ROOT/build-flatpak" \
  "$MANIFEST"

# The bundle is only a release artifact if the ref that went into it is the
# arch we were asked for — the ref name itself records it.
REFS="$(ostree refs --repo="$ROOT/flatpak-repo")"
if ! printf '%s\n' "$REFS" | grep -qx "app/$APP_ID/$ARCH/$BRANCH"; then
    echo "package-flatpak.sh: expected ref app/$APP_ID/$ARCH/$BRANCH not in repo" >&2
    printf '%s\n' "$REFS" >&2
    exit 1
fi
echo "package-flatpak.sh: repo ref confirmed: app/$APP_ID/$ARCH/$BRANCH"

mkdir -p "$OUT"
flatpak build-bundle \
  --arch="$ARCH" \
  "$ROOT/flatpak-repo" \
  "$OUT/gamehandler-v$VERSION-linux-$ARCH.flatpak" \
  "$APP_ID" \
  "$BRANCH"

echo "Created $OUT/gamehandler-v$VERSION-linux-$ARCH.flatpak"
