#!/usr/bin/env bash
#
# verify-release.sh — check that a directory holds the complete release set.
#
#   scripts/verify-release.sh dist/
#   scripts/verify-release.sh --version 0.8.0 dist/
#
# For the version's two architectures the directory must contain:
#
#   gamehandler-v<VERSION>-linux-x86_64.tar.gz
#   gamehandler-v<VERSION>-linux-aarch64.tar.gz
#   gamehandler-v<VERSION>-linux-x86_64.flatpak
#   gamehandler-v<VERSION>-linux-aarch64.flatpak
#   SHA256SUMS                              (covering exactly those four)
#
# Each tarball is opened, its bin/gamehandler's ELF machine is read with
# readelf and compared to the arch in the filename, and the staged layout
# (bin/, share/applications, share/icons, LICENSE, install.sh) is checked.
# Each .flatpak is imported into a throwaway OSTree repo and the ref's arch
# is read back — that is the bundle's own metadata, not the filename's claim.
# SHA256SUMS must verify against the four files and list nothing else.
#
# Exit 0 only when every check above passed.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ID="com.goshapps.GameHandler"
BRANCH="stable"
NAME="gamehandler"

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
usage: scripts/verify-release.sh [--version VERSION] DIR

  --version   expected release version (default: workspace Cargo.toml)
  DIR         directory holding the release artifacts
EOF
}

VERSION="" DIR=""
while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="${2:?--version needs a value}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        -*) echo "verify-release.sh: unknown option: $1" >&2; exit 2 ;;
        *) DIR="$1"; shift ;;
    esac
done
[ -n "$DIR" ] && [ -d "$DIR" ] || { echo "verify-release.sh: DIR must be an existing directory" >&2; exit 2; }
DIR="$(cd "$DIR" && pwd)"

if [ -z "$VERSION" ]; then
    VERSION="$(version_of "$ROOT/crates/app/Cargo.toml" 2>/dev/null || true)"
    [ -n "$VERSION" ] || VERSION="$(version_of "$ROOT/Cargo.toml")"
fi
[ -n "$VERSION" ] || { echo "verify-release.sh: could not determine the version" >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

fail()  { echo "FAIL $*"; FAILURES=$((FAILURES+1)); }
ok()    { echo "ok   $*"; }
have()  { command -v "$1" >/dev/null 2>&1; }

require_file() {
    local f="$DIR/$1"
    if [ ! -f "$f" ]; then fail "missing: $1"; return 1; fi
    if [ ! -s "$f" ]; then fail "zero bytes: $1"; return 1; fi
    ok "$1 ($(stat -c%s "$f") bytes)"
    return 0
}

check_tarball() {
    local arch="$1" want_machine="$2"
    local base="$NAME-v$VERSION-linux-$arch"
    local tgz="$DIR/$base.tar.gz"
    require_file "$base.tar.gz" || return 0

    if ! tar -tzf "$tgz" >"$WORK/$arch.list" 2>"$WORK/$arch.tar-err"; then
        fail "$base.tar.gz: not a readable gzip tar ($(head -1 "$WORK/$arch.tar-err"))"
        return 0
    fi
    # The top level must be exactly the one staging directory.
    local top
    top="$(awk -F/ 'NF>1 {print $1} NF==1 && $1!="" {print $1}' "$WORK/$arch.list" | sort -u)"
    if [ "$top" != "$base" ]; then
        fail "$base.tar.gz: top-level entry is '$top', expected '$base'"
    fi
    for member in \
        "$base/bin/gamehandler" \
        "$base/install.sh" \
        "$base/LICENSE" \
        "$base/share/applications/$APP_ID.desktop" \
        "$base/share/metainfo/$APP_ID.metainfo.xml" \
        "$base/share/icons/hicolor/scalable/apps/$APP_ID.svg" \
        "$base/share/gamehandler/microsoft-identity-verification-root-ca-2020.pem"; do
        grep -qx "$member" "$WORK/$arch.list" || fail "$base.tar.gz: missing member $member"
    done

    if ! tar -xzf "$tgz" -C "$WORK" "$base/bin/gamehandler" 2>/dev/null; then
        fail "$base.tar.gz: cannot extract bin/gamehandler"
        return 0
    fi
    local machine
    machine="$(readelf -h "$WORK/$base/bin/gamehandler" | awk -F: '/Machine:/ {sub(/^[ \t]+/, "", $2); print $2}')"
    if [ "$machine" = "$want_machine" ]; then
        ok "$base.tar.gz: binary is $machine"
    else
        fail "$base.tar.gz: binary is '$machine', expected '$want_machine'"
    fi
    rm -rf "$WORK/$base"
}

check_flatpak() {
    local arch="$1"
    local bundle="gamehandler-v$VERSION-linux-$arch.flatpak"
    require_file "$bundle" || return 0

    if ! have flatpak || ! have ostree; then
        fail "$bundle: flatpak/ostree not installed — cannot verify the bundle's arch"
        return 0
    fi
    local repo="$WORK/repo-$arch"
    if ! ostree init --repo="$repo" --mode=archive-z2 >/dev/null 2>&1 \
        || ! flatpak build-import-bundle --no-summary-index "$repo" "$DIR/$bundle" >/dev/null 2>&1; then
        fail "$bundle: not an importable ostree bundle"
        return 0
    fi
    # The imported ref names its arch — a bundle built for x86_64 imports as
    # app/<id>/x86_64/stable no matter what the file is called.
    local refs want="app/$APP_ID/$arch/$BRANCH"
    refs="$(ostree refs --repo="$repo")"
    if printf '%s\n' "$refs" | grep -qx "$want"; then
        ok "$bundle: ref arch is $arch"
    else
        fail "$bundle: contains '${refs:-no refs}', expected '$want'"
    fi
}

echo "verify-release.sh: version $VERSION, dir $DIR"

check_tarball x86_64  "Advanced Micro Devices X86-64"
check_tarball aarch64 "AArch64"
check_flatpak x86_64
check_flatpak aarch64

# SHA256SUMS: must exist, verify, and name exactly the four artifacts.
if require_file SHA256SUMS; then
    ( cd "$DIR" && sha256sum --check --status SHA256SUMS ) \
        && ok "SHA256SUMS verifies against the files it names" \
        || fail "SHA256SUMS does not verify"
    listed="$(awk '{print $2}' "$DIR/SHA256SUMS" | LC_ALL=C sort)"
    expected="$(printf 'gamehandler-v%s-linux-aarch64.flatpak\ngamehandler-v%s-linux-aarch64.tar.gz\ngamehandler-v%s-linux-x86_64.flatpak\ngamehandler-v%s-linux-x86_64.tar.gz\n' \
        "$VERSION" "$VERSION" "$VERSION" "$VERSION")"
    [ "$listed" = "$expected" ] || {
        fail "SHA256SUMS names: $(echo "$listed" | tr '\n' ' ')"
        fail "expected exactly: $(echo "$expected" | tr '\n' ' ')"
    }
fi

echo
if [ "$FAILURES" -eq 0 ]; then
    echo "verify-release.sh: PASS — the complete release set for v$VERSION is present and correct"
else
    echo "verify-release.sh: FAIL — $FAILURES check(s) failed"
fi
exit "$([ "$FAILURES" -eq 0 ] && echo 0 || echo 1)"
