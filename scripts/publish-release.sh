#!/usr/bin/env bash
#
# publish-release.sh — create-or-update the GitHub Release for a tag and
# attach the verified artifact set. Idempotent: safe on a workflow rerun.
#
#   scripts/publish-release.sh v0.8.0 dist/
#
# Order of operations — a release nobody can partially see:
#
#   1. verify-release.sh must pass on DIR first (complete set, right arches)
#   2. SHA256SUMS is (re)generated here so it describes exactly the files
#      being uploaded
#   3. a DRAFT release is created (or the existing one reused) — nothing is
#      public while assets are still landing
#   4. all five assets are uploaded with --clobber (rerun-safe)
#   5. the release is read back: every expected asset must be present with a
#      non-zero size
#   6. only then is the draft published
#
# A rerun where the release is already published still uploads --clobber and
# re-verifies — that is how an interrupted upload is repaired.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

[ $# -eq 2 ] || { echo "usage: scripts/publish-release.sh TAG DIR" >&2; exit 2; }
TAG="$1" DIR="$2"
VERSION="${TAG#v}"

command -v gh >/dev/null 2>&1 || { echo "publish-release.sh: gh CLI is required" >&2; exit 1; }

"$ROOT/scripts/verify-release.sh" --version "$VERSION" "$DIR" || {
    echo "publish-release.sh: refusing to publish — the artifact set failed verification" >&2
    exit 1
}

# Regenerate SHA256SUMS against exactly the four artifacts being shipped.
( cd "$DIR" && rm -f SHA256SUMS && \
  sha256sum "gamehandler-v$VERSION-linux-x86_64.tar.gz" \
            "gamehandler-v$VERSION-linux-aarch64.tar.gz" \
            "gamehandler-v$VERSION-linux-x86_64.flatpak" \
            "gamehandler-v$VERSION-linux-aarch64.flatpak" > SHA256SUMS )
"$ROOT/scripts/verify-release.sh" --version "$VERSION" "$DIR" >/dev/null || {
    echo "publish-release.sh: verification failed after SHA256SUMS regeneration" >&2
    exit 1
}

NOTES="$DIR/.release-notes.md"
"$ROOT/scripts/release-notes.sh" "$VERSION" > "$NOTES"

if gh release view "$TAG" >/dev/null 2>&1; then
    # A rerun against a still-published release would otherwise mutate it live:
    # --clobber deletes+re-uploads each asset in sequence, a window where users
    # see a partial set. Re-draft first so the window is unpublished, then the
    # read-back gate below decides whether it goes public again.
    if [ "$(gh release view "$TAG" --json isDraft --jq .isDraft)" = "false" ]; then
        echo "publish-release.sh: release $TAG is live — re-drafting while assets are replaced"
        gh release edit "$TAG" --draft=true
    else
        echo "publish-release.sh: draft release $TAG exists — resuming it"
    fi
    gh release edit "$TAG" --title "GameHandler $TAG" --notes-file "$NOTES"
else
    gh release create "$TAG" --draft --title "GameHandler $TAG" --notes-file "$NOTES"
    echo "publish-release.sh: created draft release $TAG"
fi

gh release upload "$TAG" --clobber \
    "$DIR/gamehandler-v$VERSION-linux-x86_64.tar.gz" \
    "$DIR/gamehandler-v$VERSION-linux-aarch64.tar.gz" \
    "$DIR/gamehandler-v$VERSION-linux-x86_64.flatpak" \
    "$DIR/gamehandler-v$VERSION-linux-aarch64.flatpak" \
    "$DIR/SHA256SUMS"

# Read-back gate: the release is not published until every expected asset is
# confirmed attached with a non-zero size.
missing=0
assets="$(gh release view "$TAG" --json assets --jq '[.assets[] | {name, size}]')"
for want in \
    "gamehandler-v$VERSION-linux-x86_64.tar.gz" \
    "gamehandler-v$VERSION-linux-aarch64.tar.gz" \
    "gamehandler-v$VERSION-linux-x86_64.flatpak" \
    "gamehandler-v$VERSION-linux-aarch64.flatpak" \
    "SHA256SUMS"; do
    size="$(echo "$assets" | python3 -c "import json,sys; print(next((a['size'] for a in json.load(sys.stdin) if a['name']=='$want'), 0))")"
    if [ "${size:-0}" -gt 0 ]; then
        echo "ok   asset $want ($size bytes)"
    else
        echo "FAIL asset $want missing or empty" >&2
        missing=1
    fi
done
[ "$missing" -eq 0 ] || { echo "publish-release.sh: asset check failed — leaving the release unpublished" >&2; exit 1; }

# `latest` belongs to the newest version tag, not the most recently run
# workflow — re-releasing an old tag (workflow_dispatch exists for that) must
# not steal the pointer from a newer release.
latest_flag="--latest=false"
newest="$(git ls-remote --tags origin 'v*' 2>/dev/null \
    | awk -F/ '{print $NF}' | sort -V | tail -1)"
if [ "$newest" = "$TAG" ]; then
    latest_flag="--latest"
fi

gh release edit "$TAG" --draft=false "$latest_flag"
echo "publish-release.sh: $TAG published with all assets ($latest_flag)"
