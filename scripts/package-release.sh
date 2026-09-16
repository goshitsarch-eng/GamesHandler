#!/usr/bin/env bash
#
# package-release.sh — assemble the distributable tar.gz for one architecture.
#
# The archive is a staged layout, not a dump of target/: the binary under bin/,
# the same desktop/metainfo/icon files the Flatpak installs, the licence, and a
# small install.sh that copies them into a prefix (default ~/.local). The stage
# directory is removed afterwards; only the tarball under dist/ remains.
#
#   scripts/package-release.sh --arch x86_64
#   scripts/package-release.sh --arch aarch64 --version 0.8.0
#
# Requires target/release/gamehandler to already exist — the caller builds it
# (cargo build --release --locked) so this script stays usable on a tree that
# was built by hand, by CI, or by flatpak-builder.
#
# The binary's ELF machine is checked against --arch with readelf before
# packaging: an x86_64 binary packaged under an aarch64 name — or the reverse —
# fails here rather than on a user's machine.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ID="com.goshapps.GameHandler"
NAME="gamehandler"

usage() {
    cat <<'EOF'
usage: scripts/package-release.sh --arch ARCH [--version VERSION] [--out DIR]

  --arch      x86_64 or aarch64; must match the ELF machine of the built binary
  --version   release version (default: the workspace version in Cargo.toml)
  --out       output directory for the tarball (default: dist/)
EOF
}

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

ARCH="" VERSION="" OUT="$ROOT/dist"
while [ $# -gt 0 ]; do
    case "$1" in
        --arch)    ARCH="${2:?--arch needs a value}"; shift 2 ;;
        --version) VERSION="${2:?--version needs a value}"; shift 2 ;;
        --out)     OUT="${2:?--out needs a value}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "package-release.sh: unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

case "$ARCH" in
    x86_64)  ELF_MACHINE="Advanced Micro Devices X86-64" ;;
    aarch64) ELF_MACHINE="AArch64" ;;
    *) echo "package-release.sh: --arch must be x86_64 or aarch64 (got: '$ARCH')" >&2; exit 2 ;;
esac

if [ -z "$VERSION" ]; then
    VERSION="$(version_of "$ROOT/crates/app/Cargo.toml" 2>/dev/null || true)"
    [ -n "$VERSION" ] || VERSION="$(version_of "$ROOT/Cargo.toml")"
fi
[ -n "$VERSION" ] || { echo "package-release.sh: could not determine the version" >&2; exit 1; }

BIN="$ROOT/target/release/$NAME"
[ -f "$BIN" ] || { echo "package-release.sh: $BIN does not exist — build it first" >&2; exit 1; }

# Architecture check on the actual binary, not the runner it was built on.
MACHINE="$(readelf -h "$BIN" | awk -F: '/Machine:/ {sub(/^[ \t]+/, "", $2); print $2}')"
echo "package-release.sh: ELF machine is '$MACHINE'"
[ "$MACHINE" = "$ELF_MACHINE" ] || {
    echo "package-release.sh: --arch $ARCH expects '$ELF_MACHINE' but the binary is '$MACHINE'" >&2
    exit 1
}

PKG="$NAME-v$VERSION-linux-$ARCH"
STAGE="$(mktemp -d)/$PKG"
trap 'rm -rf "$(dirname "$STAGE")"' EXIT

install -Dm0755 "$BIN" "$STAGE/bin/$NAME"
install -Dm0644 "$ROOT/data/$APP_ID.desktop"      "$STAGE/share/applications/$APP_ID.desktop"
install -Dm0644 "$ROOT/data/$APP_ID.metainfo.xml" "$STAGE/share/metainfo/$APP_ID.metainfo.xml"
for icon in \
    "scalable/apps/$APP_ID.svg" \
    "64x64/apps/$APP_ID.png" \
    "128x128/apps/$APP_ID.png" \
    "256x256/apps/$APP_ID.png" \
    "512x512/apps/$APP_ID.png"; do
    install -Dm0644 "$ROOT/data/icons/hicolor/$icon" "$STAGE/share/icons/hicolor/$icon"
done
install -Dm0644 "$ROOT/data/microsoft-identity-verification-root-ca-2020.pem" \
    "$STAGE/share/$NAME/microsoft-identity-verification-root-ca-2020.pem"
install -Dm0644 "$ROOT/LICENSE" "$STAGE/LICENSE"

# Install/uninstall into a prefix. Not packaged logic: it copies the staged
# files verbatim and registers nothing beyond what a desktop file needs.
cat > "$STAGE/install.sh" <<'EOF'
#!/bin/sh
# Install GameHandler into a prefix (default: ~/.local).
#   ./install.sh            user install
#   ./install.sh PREFIX     e.g. ./install.sh /usr/local
set -eu
PREFIX="${1:-$HOME/.local}"
SRC="$(cd "$(dirname "$0")" && pwd)"
install -Dm755 "$SRC/bin/gamehandler" "$PREFIX/bin/gamehandler"
for f in share/applications/com.goshapps.GameHandler.desktop \
         share/metainfo/com.goshapps.GameHandler.metainfo.xml \
         share/icons/hicolor/scalable/apps/com.goshapps.GameHandler.svg \
         share/icons/hicolor/64x64/apps/com.goshapps.GameHandler.png \
         share/icons/hicolor/128x128/apps/com.goshapps.GameHandler.png \
         share/icons/hicolor/256x256/apps/com.goshapps.GameHandler.png \
         share/icons/hicolor/512x512/apps/com.goshapps.GameHandler.png \
         share/gamehandler/microsoft-identity-verification-root-ca-2020.pem \
         LICENSE; do
    install -Dm644 "$SRC/$f" "$PREFIX/$f"
done
echo "Installed GameHandler into $PREFIX"
EOF
chmod 0755 "$STAGE/install.sh"

mkdir -p "$OUT"
tar -czf "$OUT/$PKG.tar.gz" -C "$(dirname "$STAGE")" "$PKG"
echo "Created $OUT/$PKG.tar.gz"
