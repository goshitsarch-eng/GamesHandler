#!/usr/bin/env bash
#
# GameHandler — the single verification entry point.
#
# One command to run before closing a task. Implements the stage list in
# docs/migration/packaging.md §6, extended with the two checks that doc and
# PLAN.md §7 also require (cargo-sources freshness, desktop/metainfo
# validation). Task T-17.
#
# Stages, in order:
#
#   1  build                 cargo build (workspace root)
#   2  clippy                cargo clippy --all-targets -- -D warnings
#                            (workspace root ONLY — DECISIONS D-08)
#   3  test                  cargo test, with DISPLAY/WAYLAND_DISPLAY unset
#   4  oracle-freshness      regenerate docs/migration/oracle/fixtures/ from the
#                            Python implementation and fail if the checked-in
#                            copy differs. This is what stops the Python
#                            contract from drifting silently.
#   5  python-tests          the existing Python suite must stay green
#                            (DECISIONS D-17: it is the behavioural reference
#                            the Rust port is written against)
#   6  cargo-sources         build-aux/flatpak/cargo-sources.json is fresh
#                            against Cargo.lock and covers every git source
#   7  flatpak-build         flatpak-builder builds the manifest
#   8  smoke-test            scripts/smoke-test.sh — CLI + headless GUI
#   9  desktop-metainfo      desktop-file-validate + appstreamcli validate
#
# Output contract (packaging.md §6): one machine-greppable line per stage on
# stdout — `ok <stage>`, `FAIL <stage>`, `SKIP <stage>` — and every stage is
# preceded by `### <stage>`. Full tool output goes to
# target/verify-logs/<stage>.log; the head of that log is tail-able while a
# long stage runs, only the tail is echoed on failure, and a stage that reports
# its own sub-checks (the smoke test) has those echoed indented so a pass is
# still legible without opening the log.
#
# Read-only with respect to the repository. Stage 4 regenerates the oracle into
# a temporary directory and compares, rather than writing over the tree, and
# asserts afterwards that the fixtures on disk are unchanged. Every other stage
# writes only to gitignored paths (target/, .flatpak-builder/, build-flatpak/,
# flatpak-repo/).
#
# Usage: scripts/verify.sh [options]
#
#   --skip-flatpak   skip stages 7 and 8 (fast local iteration)
#   --skip-smoke     skip stage 8 only
#   --keep-going     run every stage and report the full table instead of
#                    stopping at the first failure (packaging.md §6 specifies
#                    fail-fast; this is the flag for a full diagnostic sweep)
#   --offline        pass --disable-download to flatpak-builder
#   --hold SECONDS   forward the GUI hold interval to the smoke test
#
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ID="com.goshapps.GameHandler"
MANIFEST="$ROOT/build-aux/flatpak/com.goshapps.GameHandler.json"
BUILD_DIR="$ROOT/build-flatpak"
REPO_DIR="$ROOT/flatpak-repo"
LOGDIR="$ROOT/target/verify-logs"

SKIP_FLATPAK=0
SKIP_SMOKE=0
KEEP_GOING=0
OFFLINE=0
SMOKE_HOLD=""

PASSED=()
FAILED=()
SKIPPED=()

usage() {
    cat <<'EOF'
usage: scripts/verify.sh [options]

  --skip-flatpak   skip stages 7 and 8 (fast local iteration)
  --skip-smoke     skip stage 8 only
  --keep-going     run every stage and report the full table instead of
                   stopping at the first failure
  --offline        pass --disable-download to flatpak-builder
  --hold SECONDS   forward the GUI hold interval to the smoke test

Stages: build, clippy, test, oracle-freshness, python-tests, cargo-sources,
        flatpak-build, smoke-test, desktop-metainfo
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --skip-flatpak) SKIP_FLATPAK=1; SKIP_SMOKE=1; shift ;;
        --skip-smoke)   SKIP_SMOKE=1; shift ;;
        --keep-going)   KEEP_GOING=1; shift ;;
        --offline)      OFFLINE=1; shift ;;
        --hold)         SMOKE_HOLD="${2:?--hold needs a value}"; shift 2 ;;
        -h|--help)      usage; exit 0 ;;
        *)              echo "verify.sh: unknown option: $1" >&2; exit 2 ;;
    esac
done

mkdir -p "$LOGDIR"
printf 'logs: %s/ (tail them while a long stage runs)\n\n' "${LOGDIR#"$ROOT"/}"
cd "$ROOT"

STAGE=""
STAGE_LOG=""
STAGE_START=0

begin() {
    STAGE="$1"
    STAGE_LOG="$LOGDIR/$1.log"
    STAGE_START="$SECONDS"
    printf '### %s\n' "$STAGE"
}

# The outcome is decided by the caller; these only report it.
#
# The per-stage log goes to target/verify-logs/. Stages that report their own
# sub-checks (the smoke test) have those lines echoed indented, so a passing
# stage is still legible without opening the log. The indent matters: it keeps
# `^ok|^FAIL|^SKIP` matching only the one machine-greppable line per stage.
echo_subchecks() {
    grep -E '^(ok|FAIL|SKIP) ' "$STAGE_LOG" 2>/dev/null | sed 's/^/     /'
}

finish_ok()   {
    printf 'ok   %-18s (%ds)\n' "$STAGE" "$((SECONDS - STAGE_START))"
    echo_subchecks
    PASSED+=("$STAGE")
}
finish_skip() {
    printf 'SKIP %-18s %s\n' "$STAGE" "$1"
    echo_subchecks
    SKIPPED+=("$STAGE")
}

summary() {
    printf '\n=== summary ===\n'
    printf 'passed:  %s\n' "${PASSED[*]:-none}"
    printf 'failed:  %s\n' "${FAILED[*]:-none}"
    printf 'skipped: %s\n' "${SKIPPED[*]:-none}"
    printf 'logs:    %s/\n' "${LOGDIR#"$ROOT"/}"
    if [ "${#SKIPPED[@]}" -gt 0 ]; then
        printf 'NOTE: a SKIP is not a pass — those stages did not run.\n'
    fi
}

finish_fail() {
    printf 'FAIL %-18s (%ds)\n' "$STAGE" "$((SECONDS - STAGE_START))"
    if [ -s "$STAGE_LOG" ]; then
        printf '     --- last 30 lines of %s ---\n' "${STAGE_LOG#"$ROOT"/}"
        tail -n 30 "$STAGE_LOG" | sed 's/^/     | /'
        printf '     --- end ---\n'
    else
        printf '     | (no output captured)\n'
    fi
    FAILED+=("$STAGE")
    if [ "$KEEP_GOING" -eq 0 ]; then
        summary
        exit 1
    fi
}

# A missing tool is reported with instructions, never as an obscure failure
# (T-17: verify.sh must work from a clean checkout).
require_tool() {
    local tool="$1" why="$2"
    command -v "$tool" >/dev/null 2>&1 && return 0
    echo "missing tool: $tool ($why)"
    echo "  install it, or pass --skip-flatpak to skip the stages that need it"
    return 1
}

# ---------------------------------------------------------------------------
# Stage 1 — build
# ---------------------------------------------------------------------------
stage_build() {
    cargo build
}

# ---------------------------------------------------------------------------
# Stage 2 — clippy, at our workspace root only (DECISIONS D-08)
# ---------------------------------------------------------------------------
stage_clippy() {
    cargo clippy --all-targets -- -D warnings
}

# ---------------------------------------------------------------------------
# Stage 3 — tests, with no display, to prove the logic suite is headless
# ---------------------------------------------------------------------------
stage_test() {
    env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET cargo test
}

# ---------------------------------------------------------------------------
# Stage 4 — oracle freshness
#
# gen_oracle.py derives its repo root from its own path (parents[3]) and writes
# its fixtures next to itself. To keep this script read-only with respect to the
# repo we stage a throwaway copy of the *only* package it imports (`gamehandler`
# — the generator imports gamehandler.models and gamehandler.settings and
# nothing else) plus the generator, at the same depth, in a temp dir, so the
# generator writes there and the repo is untouched. The fixtures on disk are
# hashed before and after as a belt-and-braces assertion that this stayed true.
# ---------------------------------------------------------------------------
ORACLE_REL="docs/migration/oracle"
FIXTURES_REL="$ORACLE_REL/fixtures"

fixture_hashes() {
    find "$ROOT/$FIXTURES_REL" -type f -print0 2>/dev/null \
        | sort -z \
        | xargs -0 sha256sum 2>/dev/null
}

stage_oracle() {
    local gen="$ROOT/$ORACLE_REL/gen_oracle.py"
    [ -f "$gen" ] || { echo "no such file: $ORACLE_REL/gen_oracle.py"; return 1; }

    local before
    before="$(fixture_hashes)"

    local tmp
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/gh-oracle-XXXXXX")" || return 1
    mkdir -p "$tmp/docs/migration/oracle"
    cp -r "$ROOT/gamehandler" "$tmp/gamehandler"
    # Drop bytecode caches: not needed, and they carry whatever interpreter
    # version happened to write them.
    find "$tmp/gamehandler" -name __pycache__ -type d -prune -exec rm -rf {} + 2>/dev/null
    cp "$gen" "$tmp/docs/migration/oracle/gen_oracle.py"

    local rc=0
    ( cd "$tmp" && python3 docs/migration/oracle/gen_oracle.py ) || rc=$?
    if [ "$rc" -ne 0 ]; then
        echo "the generator failed with status $rc (see its output above)"
        rm -rf "$tmp"
        return 1
    fi

    rc=0
    diff -ru "$ROOT/$FIXTURES_REL" "$tmp/$FIXTURES_REL" || rc=$?
    if [ "$rc" -ne 0 ]; then
        echo
        echo "The checked-in oracle is STALE relative to the Python implementation."
        echo "The diff above is what regenerating produces; the checked-in copy"
        echo "comes first, the regenerated copy second."
        echo "Fix: run 'python3 $ORACLE_REL/gen_oracle.py' and commit the result."
        rm -rf "$tmp"
        return 1
    fi
    echo "fixtures match: $(find "$ROOT/$FIXTURES_REL" -type f | wc -l) files, $(du -sh "$ROOT/$FIXTURES_REL" | cut -f1)"
    rm -rf "$tmp"

    local after
    after="$(fixture_hashes)"
    if [ "$before" != "$after" ]; then
        echo "the fixtures on disk changed during this stage — it must be read-only"
        return 1
    fi
    return 0
}

# ---------------------------------------------------------------------------
# Stage 5 — the Python suite stays green (DECISIONS D-17)
# ---------------------------------------------------------------------------
stage_python() {
    env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
        python3 -m unittest discover -s tests -t . -v
}

# ---------------------------------------------------------------------------
# Stage 6 — cargo-sources.json freshness + git coverage
#
# The generator is not part of this repository (it is flatpak/flatpak-builder-
# tools' cargo/flatpak-cargo-generator.py). Look for it where it is normally
# kept; if it is nowhere, SKIP loudly — this is the one stage that needs
# something a clean checkout does not contain.
# ---------------------------------------------------------------------------
find_cargo_generator() {
    if [ -n "${FLATPAK_CARGO_GENERATOR:-}" ] && [ -f "${FLATPAK_CARGO_GENERATOR}" ]; then
        printf '%s\n' "$FLATPAK_CARGO_GENERATOR"; return 0
    fi
    if command -v flatpak-cargo-generator.py >/dev/null 2>&1; then
        command -v flatpak-cargo-generator.py; return 0
    fi
    local candidate
    for candidate in \
        "$HOME/.cache/flatpak-builder-tools/cargo/flatpak-cargo-generator.py" \
        /tmp/gh-gen/flatpak-cargo-generator.py \
        /usr/share/flatpak-builder-tools/cargo/flatpak-cargo-generator.py
    do
        [ -f "$candidate" ] && { printf '%s\n' "$candidate"; return 0; }
    done
    return 1
}

# The interpreter that can run it: the generator imports aiohttp, which a distro
# python3 usually lacks. Prefer a venv beside the script when one exists.
find_generator_python() {
    local gen="$1" dir
    dir="$(dirname "$gen")"
    if [ -x "$dir/venv/bin/python" ] && "$dir/venv/bin/python" -c 'import aiohttp' 2>/dev/null; then
        printf '%s\n' "$dir/venv/bin/python"; return 0
    fi
    if python3 -c 'import aiohttp' 2>/dev/null; then
        printf 'python3\n'; return 0
    fi
    return 1
}

# Canonical set comparison: the generator's entry order is an implementation
# detail; the entry set is the contract.
COMPARE_PY='
import json, sys
committed = {json.dumps(e, sort_keys=True) for e in json.load(open(sys.argv[1]))}
regen = {json.dumps(e, sort_keys=True) for e in json.load(open(sys.argv[2]))}
only_committed = sorted(committed - regen)
only_regen = sorted(regen - committed)
if only_committed or only_regen:
    print("cargo-sources.json is STALE relative to Cargo.lock.")
    for e in only_committed:
        print("  only in the committed copy:  " + e)
    for e in only_regen:
        print("  only in the regenerated one: " + e)
    print("Fix: regenerate and commit --")
    print("  python3 flatpak-cargo-generator.py Cargo.lock -o build-aux/flatpak/cargo-sources.json")
    sys.exit(1)
print("matches: {} sources".format(len(committed)))
'

# Does cargo-sources.json carry a type:git entry for this Cargo.lock git URL?
COVERS_PY='
import json, sys
entries = json.load(open(sys.argv[1]))
url = sys.argv[2].rstrip("/")
if url.endswith(".git"):
    url = url[:-4]
for e in entries:
    if e.get("type") != "git":
        continue
    cand = e.get("url", "").rstrip("/")
    if cand.endswith(".git"):
        cand = cand[:-4]
    if cand == url:
        sys.exit(0)
sys.exit(1)
'

stage_cargo_sources() {
    local committed="$ROOT/build-aux/flatpak/cargo-sources.json"
    [ -f "$committed" ] || { echo "no such file: build-aux/flatpak/cargo-sources.json"; return 1; }
    [ -f "$ROOT/Cargo.lock" ] || { echo "no such file: Cargo.lock"; return 1; }

    local gen
    if ! gen="$(find_cargo_generator)"; then
        echo "flatpak-cargo-generator.py not found, so cargo-sources.json cannot be checked."
        echo "Install it from flatpak/flatpak-builder-tools (cargo/), then put it on PATH"
        echo "or set FLATPAK_CARGO_GENERATOR=/path/to/flatpak-cargo-generator.py."
        return 99   # 99 => SKIP
    fi
    echo "generator:   $gen"

    local py
    if ! py="$(find_generator_python "$gen")"; then
        echo "no python interpreter with the 'aiohttp' module is available to run it."
        echo "e.g. 'python3 -m venv venv && venv/bin/pip install aiohttp' beside the script."
        return 99   # 99 => SKIP
    fi
    echo "interpreter: $py"

    local tmp
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/gh-cargsrc-XXXXXX")" || return 1
    local rc=0
    "$py" "$gen" "$ROOT/Cargo.lock" -o "$tmp/cargo-sources.json" || rc=$?
    if [ "$rc" -ne 0 ]; then
        echo "the generator failed with status $rc (see its output above)"
        rm -rf "$tmp"
        return 1
    fi

    rc=0
    python3 -c "$COMPARE_PY" "$committed" "$tmp/cargo-sources.json" || rc=$?
    rm -rf "$tmp"
    [ "$rc" -eq 0 ] || return 1

    # Every git source the lock file needs must be present. A libcosmic rev bump
    # can add one silently, and the failure it causes (an offline fetch error
    # deep inside flatpak-builder) is otherwise hard to attribute. See
    # packaging.md §2.1 and PLAN.md risk R-2.
    local urls url missing=0 count=0
    urls="$(sed -n 's/^source = "git+\([^?#]*\).*/\1/p' "$ROOT/Cargo.lock" | sort -u)"
    while IFS= read -r url; do
        [ -n "$url" ] || continue
        count=$((count + 1))
        if ! python3 -c "$COVERS_PY" "$committed" "$url"; then
            echo "Cargo.lock needs git source '$url' but cargo-sources.json has no type:git entry for it"
            missing=$((missing + 1))
        else
            echo "  covered: $url"
        fi
    done <<<"$urls"
    echo "git sources in Cargo.lock: $count, missing: $missing"
    [ "$missing" -eq 0 ] || return 1
    return 0
}

# ---------------------------------------------------------------------------
# Stage 7 — flatpak-builder
# ---------------------------------------------------------------------------
stage_flatpak() {
    local -a flags=(
        --user
        --force-clean
        --disable-rofiles-fuse
        --install-deps-from=flathub
        --default-branch=stable
        --state-dir="$ROOT/.flatpak-builder"
        --repo="$REPO_DIR"
    )
    # packaging.md §6 called this --disable-network; flatpak-builder 1.4.10 has
    # no such flag, it has --disable-download.
    [ "$OFFLINE" -eq 1 ] && flags+=(--disable-download)
    flatpak-builder "${flags[@]}" "$BUILD_DIR" "$MANIFEST"
}

# ---------------------------------------------------------------------------
# Stage 8 — headless smoke test (scripts/smoke-test.sh)
# ---------------------------------------------------------------------------
stage_smoke() {
    local -a args=(--build-dir "$BUILD_DIR")
    [ -n "$SMOKE_HOLD" ] && args+=(--hold "$SMOKE_HOLD")
    local rc=0
    "$ROOT/scripts/smoke-test.sh" "${args[@]}" || rc=$?
    # 0 = every runnable check passed, 77 = only the CLI checks ran because no
    # compositor was available. Anything else is a failure, reported as one.
    if [ "$rc" -eq 77 ]; then return 77; fi
    return "$rc"
}

# ---------------------------------------------------------------------------
# Stage 9 — desktop entry and AppStream metadata
# ---------------------------------------------------------------------------
stage_desktop_metainfo() {
    local share="$BUILD_DIR/files/share"
    local desktop="$share/applications/$APP_ID.desktop"
    local metainfo="$share/metainfo/$APP_ID.metainfo.xml"
    [ -f "$desktop" ] || desktop="$ROOT/data/$APP_ID.desktop"
    [ -f "$metainfo" ] || metainfo="$ROOT/data/$APP_ID.metainfo.xml"
    echo "desktop:  ${desktop#"$ROOT"/}"
    echo "metainfo: ${metainfo#"$ROOT"/}"

    local rc=0 missing=0
    if require_tool desktop-file-validate "from desktop-file-utils"; then
        desktop-file-validate "$desktop" || rc=1
    else
        missing=1
    fi
    if require_tool appstreamcli "from appstream"; then
        appstreamcli validate --no-net "$metainfo" || rc=1
    else
        missing=1
    fi
    # An invalid file is a failure; a missing validator is not one (the repo is
    # not broken, the tool is absent) -- but the stage genuinely did not run, so
    # it must report SKIP rather than ok. Silence would be a fake pass.
    if [ "$rc" -ne 0 ]; then return 1; fi
    if [ "$missing" -ne 0 ]; then return 99; fi
    return 0
}

# ---------------------------------------------------------------------------
# Runner
# ---------------------------------------------------------------------------

# Recorded so the script can assert it did not dirty the tree (see below).
STATUS_BEFORE="$(git status --porcelain 2>/dev/null)"

run_stage() {
    local name="$1" fn="$2"
    begin "$name"
    local rc=0
    "$fn" >"$STAGE_LOG" 2>&1 || rc=$?
    case "$rc" in
        0)  finish_ok ;;
        99) finish_skip "prerequisite missing — see ${STAGE_LOG#"$ROOT"/}" ;;
        77) finish_skip "no display available — see ${STAGE_LOG#"$ROOT"/}" ;;
        *)  finish_fail ;;
    esac
}

run_stage build             stage_build
run_stage clippy            stage_clippy
run_stage test              stage_test
run_stage oracle-freshness  stage_oracle
run_stage python-tests      stage_python
run_stage cargo-sources     stage_cargo_sources

if [ "$SKIP_FLATPAK" -eq 1 ]; then
    begin flatpak-build; finish_skip "--skip-flatpak"
elif require_tool flatpak-builder "from flatpak-builder"; then
    run_stage flatpak-build stage_flatpak
else
    begin flatpak-build; finish_skip "flatpak-builder is not installed"
fi

if [ "$SKIP_SMOKE" -eq 1 ]; then
    begin smoke-test; finish_skip "--skip-flatpak/--skip-smoke"
else
    run_stage smoke-test stage_smoke
fi

if [ "$SKIP_FLATPAK" -eq 1 ] && [ ! -d "$BUILD_DIR/files" ]; then
    begin desktop-metainfo
    finish_skip "no build tree — run without --skip-flatpak to validate the installed copies"
else
    run_stage desktop-metainfo stage_desktop_metainfo
fi

# The repository must be as clean after a run as before it (T-17 constraint).
STATUS_AFTER="$(git status --porcelain 2>/dev/null)"
summary
if [ "$STATUS_BEFORE" != "$STATUS_AFTER" ]; then
    printf '\nNOTE: the working tree changed during this run. verify.sh must be\n'
    printf 'read-only with respect to tracked files — please investigate:\n'
    diff <(printf '%s\n' "$STATUS_BEFORE") <(printf '%s\n' "$STATUS_AFTER") | sed 's/^/  /'
fi

if [ "${#FAILED[@]}" -gt 0 ]; then
    exit 1
fi
exit 0
