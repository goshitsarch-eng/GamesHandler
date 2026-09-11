#!/usr/bin/env bash
#
# GameHandler — the single verification entry point.
#
# One command to run before closing a task. Implements the stage list in
# docs/migration/packaging.md §6, extended with the three checks that doc and
# PLAN.md §7 also require (cargo-sources freshness, desktop/metainfo
# validation, and — T-23 — that the built Flatpak actually carries the files no
# validator looks at). Task T-17.
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
#  10  flatpak-contents      the built Flatpak carries the files no validator
#                            looks at: the application's own licence text, the
#                            Authenticode trust root, and the three metadata
#                            installs — which stage 9 reads for well-formedness
#                            (two of them) but never for presence or path.
#                            Added by T-23, after the GPL text turned out to be
#                            missing from the Flatpak with every one of stages
#                            1-9 green; widened to the metadata installs and to
#                            the manifest's whole install set by tasks #23/#24.
#
# Stages 7-10 are one critical section under a `flock`, because all four read or
# write build-flatpak/ and stage 7 writes it destructively. The lock note above
# `acquire_flatpak_lock` explains why the lock spans the group rather than each
# stage; the short version is that a released lock in the middle is a window for
# another run to swap the tree, and the reader would then validate a different
# revision with every line still saying "ok".
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
# Exit codes (DECISIONS D-31):
#
#   0  every stage either passed or was skipped *because the caller asked*
#      (--skip-flatpak / --skip-smoke). A requested skip is a legitimate pass:
#      those flags exist for fast local iteration, and a flag that skipped work
#      and then failed would be useless for the loop it is documented for.
#   1  a stage failed. A defect to fix.
#   2  usage error (unknown option), reported before any stage runs.
#   3  the run was INCOMPLETE: a stage was skipped for a missing prerequisite
#      rather than by request. Everything that ran may still be green, but this
#      run did not verify what its stage list claims — most sharply, without
#      flatpak-builder the Flatpak is never built or inspected, which is how
#      "verify.sh passes from a clean checkout" (PLAN.md §9) could otherwise be
#      satisfied by a run that never built anything.
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
# Split by cause, because only one of them is a legitimate exit 0: see the
# "Exit codes" note in the header. `finish_skip` fills these.
SKIPPED_REQUESTED=()
SKIPPED_UNREQUESTED=()

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
        flatpak-build, smoke-test, desktop-metainfo, flatpak-contents
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
# finish_skip <reason> [requested]
#
# `requested` is 1 only when the user asked for this skip (--skip-flatpak /
# --skip-smoke) and 0 (the default) when a prerequisite was missing instead.
# The two are not the same thing and the exit code treats them differently —
# see the "Exit codes" note in the header. The default is the *conservative*
# direction on purpose: forgetting to mark a requested skip makes the run look
# incomplete, which is noisy but safe, whereas the opposite default would let an
# unrequested skip hide inside an exit 0.
finish_skip() {
    local reason="$1" requested="${2:-0}"
    printf 'SKIP %-18s %s\n' "$STAGE" "$reason"
    echo_subchecks
    SKIPPED+=("$STAGE")
    if [ "$requested" -eq 1 ]; then
        SKIPPED_REQUESTED+=("$STAGE")
    else
        SKIPPED_UNREQUESTED+=("$STAGE")
    fi
}

summary() {
    printf '\n=== summary ===\n'
    printf 'passed:  %s\n' "${PASSED[*]:-none}"
    printf 'failed:  %s\n' "${FAILED[*]:-none}"
    printf 'skipped: %s\n' "${SKIPPED[*]:-none}"
    printf 'logs:    %s/\n' "${LOGDIR#"$ROOT"/}"
    if [ "${#SKIPPED_REQUESTED[@]}" -gt 0 ]; then
        printf 'asked to skip: %s (legitimate)\n' "${SKIPPED_REQUESTED[*]}"
    fi
    if [ "${#SKIPPED_UNREQUESTED[@]}" -gt 0 ]; then
        printf '\nNOTE: %s did not run for a missing prerequisite, so this run did\n' \
            "${SKIPPED_UNREQUESTED[*]}"
        printf 'NOT verify what the DoD claims it verifies. A SKIP is not a pass,\n'
        printf 'and this is not an exit 0 — see the reason above each SKIP line.\n'
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

# ---------------------------------------------------------------------------
# The Flatpak build lock
#
# Stages 7-10 all touch build-flatpak/: 7 writes it destructively (--force-clean
# erases the tree), 8 runs the app out of it, and 9 and 10 read the installed
# copies from it. Two verify.sh runs in one checkout therefore corrupt each
# other — one wipes the tree while the other is reading it, which makes
# flatpak-contents report SKIP. That is the worst possible shape for this
# harness: the reader concludes the check does not work, when what actually
# happened is that it was raced. A result that depends on who else is running is
# not a gate.
#
# ONE lock spans the whole group, and that is the load-bearing detail. Taking
# and releasing per stage would leave a window between "I built it" and "I read
# it" in which the other run completes its own build, so the reader validates a
# tree belonging to a different revision while every line of output still says
# "ok" — silent, and worse than the SKIP it replaces. The lock has to span
# "built it" through "read it", not each stage separately.
#
# The lockfile is under target/, which is gitignored, and specifically NOT under
# build-flatpak/ or .flatpak-builder/: flatpak-builder --force-clean removes
# those itself, and deleting a lock file defeats the lock, because a later run
# then locks a fresh inode and both proceed.
# ---------------------------------------------------------------------------
FLATPAK_LOCK="$ROOT/target/verify-flatpak.lock"
FLATPAK_LOCK_FD=""

acquire_flatpak_lock() {
    mkdir -p "$(dirname "$FLATPAK_LOCK")" || return 1
    if ! command -v flock >/dev/null 2>&1; then
        # Reported rather than fatal, and reported rather than silent: the run
        # can still do its work, but its result now depends on who else is
        # running, and a reader has to be told that to interpret it. Failing
        # hard here would take stages 1-6 down with it on a box without
        # util-linux, which would be a worse trade — those stages are unaffected
        # by this hazard.
        echo "flock is not installed, so stages 7-10 are NOT serialised against"
        echo "  another verify.sh in this checkout. If none is running, the result"
        echo "  is sound; if one is, build-flatpak/ may have been swapped underneath"
        echo "  this run. Install util-linux (flock) to remove the caveat."
        return 0
    fi
    exec {FLATPAK_LOCK_FD}>>"$FLATPAK_LOCK" || return 1
    # Non-blocking first, purely so that contention can be *reported*. These
    # stages take minutes, so a silent wait is indistinguishable from a hang.
    if ! flock -n "$FLATPAK_LOCK_FD"; then
        printf '     waiting for the Flatpak build lock (%s)\n' "${FLATPAK_LOCK#"$ROOT"/}"
        printf '     another verify.sh is building or reading build-flatpak/; resuming when it releases\n'
        flock "$FLATPAK_LOCK_FD" || return 1
    fi
    return 0
}

# Idempotent, and safe to call from the EXIT trap as well as directly.
release_flatpak_lock() {
    [ -n "$FLATPAK_LOCK_FD" ] || return 0
    eval "exec ${FLATPAK_LOCK_FD}>&-"
    FLATPAK_LOCK_FD=""
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
# TWO generators write into one fixture directory, and this stage must run both:
#
#   gen_oracle.py           oracle.json and the *.in.json / *.out.json pairs
#                           (the file-shaped fixtures)
#   run_runners_vectors.py  runners_vectors.cases.json and .answers.json
#                           (the function-shaped ones; --suite writes the case
#                           list, the answers are that list fed back in)
#
# Both derive their repo root from their own path (parents[3]) and write next to
# themselves. To keep this script read-only with respect to the repo we stage a
# throwaway copy of the *only* package they import (`gamehandler` — gen_oracle
# imports gamehandler.models and gamehandler.settings, run_runners_vectors
# imports gamehandler.runners and gamehandler.models, and nothing else between
# them) plus both generators, at the same depth, in a temp dir, so the
# generators write there and the repo is untouched. The fixtures on disk are
# hashed before and after as a belt-and-braces assertion that this stayed true.
#
# Running only the first — which is what this stage did from T-17 until task
# #26 — leaves the second generator's two outputs permanently `Only in <repo>`,
# so the diff can never be clean and the stage fails unconditionally. It landed
# that way in 8df5ed8 (the stage) and c72e2e1 (the fixtures, the same commit
# that added the second generator) and stayed that way for every commit since:
# read as "the oracle is stale" rather than as "this check is broken", which is
# the failure mode the check exists to prevent, with the sign flipped.
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
    local vectors="$ROOT/$ORACLE_REL/run_runners_vectors.py"
    [ -f "$gen" ] || { echo "no such file: $ORACLE_REL/gen_oracle.py"; return 1; }
    [ -f "$vectors" ] || { echo "no such file: $ORACLE_REL/run_runners_vectors.py"; return 1; }

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
    cp "$vectors" "$tmp/docs/migration/oracle/run_runners_vectors.py"

    # Both generators, in this order: gen_oracle.py creates fixtures/, which the
    # two redirections below write into. The interpreters are the same python3
    # the rest of the stage uses; neither generator needs aiohttp (that is the
    # cargo-sources generator, stage 6), the app, or the network.
    local rc=0
    (
        cd "$tmp" || exit 1
        python3 docs/migration/oracle/gen_oracle.py || exit $?
        python3 docs/migration/oracle/run_runners_vectors.py --suite \
            >docs/migration/oracle/fixtures/runners_vectors.cases.json || exit $?
        python3 docs/migration/oracle/run_runners_vectors.py \
            <docs/migration/oracle/fixtures/runners_vectors.cases.json \
            >docs/migration/oracle/fixtures/runners_vectors.answers.json || exit $?
    ) || rc=$?
    if [ "$rc" -ne 0 ]; then
        echo "a generator failed with status $rc (see its output above)"
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
        echo "Fix: regenerate with BOTH generators and commit the result --"
        echo "  python3 $ORACLE_REL/gen_oracle.py"
        echo "  python3 $ORACLE_REL/run_runners_vectors.py --suite \\"
        echo "      > $FIXTURES_REL/runners_vectors.cases.json"
        echo "  python3 $ORACLE_REL/run_runners_vectors.py \\"
        echo "      < $FIXTURES_REL/runners_vectors.cases.json \\"
        echo "      > $FIXTURES_REL/runners_vectors.answers.json"
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
# Stage 10 — what the build installs that no validator looks at
#
# Stage 9 loads the desktop entry and the metainfo and asks whether they are
# *well-formed*. Nothing in stages 1-9 asks whether the Flatpak contains them —
# or the application's own licence, or the Authenticode trust root, or the icon
# — because none of those files is an input to a validator. That gap is not
# hypothetical: T-16 moved the build from meson (which ran data/meson.build's
# install_data) to cargo, dropped the licence install with it, and every stage
# stayed green while the Flatpak shipped a GPL-3 binary with no licence. T-23.
#
# The list is every file the gamehandler module installs *except the binary*,
# and what covers each one before this stage gets to it:
#
#   LICENSE, the trust root   nothing at all.
#   desktop entry, metainfo   stage 9 reads their *content* for well-formedness
#                             (on the built copy when there is one). Nothing read
#                             the manifest to see that they are installed, or
#                             where. An installed path is a name, and no
#                             validator compares a name to the app id the window
#                             reports — D-28's hole, one level over.
#   the icon                  nothing. Not "nothing for its destination": nothing
#                             at all, from any stage (task #23). Delete its
#                             install line and ten stages stay green while the
#                             desktop file's Icon= resolves to nothing.
#
# T-23's commit message ends "The three metadata installs stay covered by stage
# 9." That is off by one — stage 9 loads two of the three, and never their
# installed paths — and it is corrected here, next to the count it gets wrong,
# rather than rewritten into that message, which is history.
#
# Two halves, deliberately:
#
#   1. the manifest declares an install for each file, *and* declares no install
#      in this module that the list above omits. This needs no build tree, so it
#      still runs when flatpak-builder cannot (an offline vendoring gap, say) —
#      the check that would have caught the original defect must not be hostage
#      to the build succeeding. It reads the manifest rather than restating what
#      it says: the assertion is "some command in here installs to this
#      destination", and it fails if that command is deleted. The completeness
#      half is what makes the list a check rather than a memory: adding an
#      install to the manifest without adding it to the list is a failure here.
#   2. when a build tree exists, the installed bytes are compared against the
#      repository's own copy. Byte-identity is the point — an install line that
#      points at the wrong file passes half 1 and fails here.
#
# A missing build tree is reported as SKIP, never as a pass: the sub-check lines
# still print, so a SKIP that got half-way is legible. Which *kind* of SKIP it is
# matters to the exit code (D-31): with --skip-flatpak the absent tree is the
# flag's own consequence and costs nothing, so the stage returns 98 rather than
# 99 there.
# ---------------------------------------------------------------------------

# Does the manifest install <src> to <dest>?
#
# Matched on *arguments*, not on a substring of the command line (task #22). The
# substring version accepted two false passes, both measured against it:
#
#   cp LICENSE /tmp/x && install -Dm0644 COPYING ${FLATPAK_DEST}/.../LICENSE
#   : install -Dm0644 LICENSE ${FLATPAK_DEST}/.../LICENSE
#
# The first installs the wrong file; the second is a shell no-op that installs
# nothing at all. Both were reported as "ok manifest installs LICENSE to ...".
# So: the command must *begin* with `install` (an install chained onto another
# command with `&&` is therefore not matched — it reads as a failure, which is
# the safe direction, and the manifest's convention is one install per
# build-command), and the token immediately before the destination token must be
# exactly the source. ${FLATPAK_DEST}/... and /app/... spellings are accepted by
# matching on the destination tail. It is still a string match on a command and
# not an execution of it; it is now a match on the argument that decides the
# outcome rather than on any substring that happens to appear.
DECLARES_PY='
import json, shlex, sys
doc = json.load(open(sys.argv[1]))
src, dest = sys.argv[2], sys.argv[3]

def is_dest(token):
    return token == dest or token.endswith("/" + dest)

for module in doc.get("modules", []):
    commands = module.get("build-commands", []) + module.get("post-install", [])
    for command in commands:
        if not command.startswith("install "):
            continue
        try:
            tokens = shlex.split(command)
        except ValueError:
            continue
        for i, token in enumerate(tokens):
            if i >= 1 and is_dest(token) and tokens[i - 1] == src:
                sys.exit(0)
sys.exit(1)
'

# Is every install in the gamehandler module covered by FLATPAK_CONTENTS?
#
# The one thing a hardcoded list cannot do is notice an addition to the thing it
# lists. This reads the module's own install commands and fails for any
# destination that is neither in the list nor the binary (which stages 7-8
# exercise by running it). Called with the destinations as arguments, so the list
# stays the single source of truth for what is covered.
COMPLETENESS_PY='
import json, shlex, sys
doc = json.load(open(sys.argv[1]))
covered = set(sys.argv[2:])
MODULE = "gamehandler"
EXCEPT = {"bin/gamehandler"}

def rel(token):
    for prefix in ("${FLATPAK_DEST}/", "/app/"):
        if token.startswith(prefix):
            return token[len(prefix):]
    return None

installs = []
for module in doc.get("modules", []):
    if module.get("name") != MODULE:
        continue
    commands = module.get("build-commands", []) + module.get("post-install", [])
    for command in commands:
        if not command.startswith("install "):
            continue
        try:
            tokens = shlex.split(command)
        except ValueError:
            continue
        destination = rel(tokens[-1]) if tokens else None
        if destination is not None:
            installs.append(destination)

uncovered = sorted({d for d in installs if d not in covered and d not in EXCEPT})
for destination in uncovered:
    print("the manifest installs " + destination + " and no entry in this")
    print("stage covers it -- add it here, or say in the list why it is exempt")
sys.exit(1 if uncovered else 0)
'

# <repository path>|<path under /app> — every file the gamehandler module
# installs except the binary. Kept in step with the manifest by the completeness
# check above, not by memory.
FLATPAK_CONTENTS=(
    "LICENSE|share/licenses/$APP_ID/LICENSE"
    "data/microsoft-identity-verification-root-ca-2020.pem|share/gamehandler/microsoft-identity-verification-root-ca-2020.pem"
    "data/$APP_ID.desktop|share/applications/$APP_ID.desktop"
    "data/$APP_ID.metainfo.xml|share/metainfo/$APP_ID.metainfo.xml"
    "data/icons/hicolor/scalable/apps/$APP_ID.svg|share/icons/hicolor/scalable/apps/$APP_ID.svg"
)

stage_flatpak_contents() {
    local rc=0 entry src dest
    local -a covered=()
    for entry in "${FLATPAK_CONTENTS[@]}"; do covered+=("${entry#*|}"); done

    echo "manifest: ${MANIFEST#"$ROOT"/}"
    if python3 -c "$COMPLETENESS_PY" "$MANIFEST" "${covered[@]}"; then
        echo "ok   every install in the gamehandler module is covered here (${#covered[@]} files)"
    else
        echo "FAIL the manifest installs a file this stage does not cover"
        rc=1
    fi

    for entry in "${FLATPAK_CONTENTS[@]}"; do
        src="${entry%%|*}"
        dest="${entry#*|}"
        if [ ! -f "$ROOT/$src" ]; then
            echo "FAIL $src is not in the repository, so the build cannot install it"
            rc=1
            continue
        fi
        if python3 -c "$DECLARES_PY" "$MANIFEST" "$src" "$dest"; then
            echo "ok   manifest installs $src to $dest"
        else
            echo "FAIL the manifest never installs $src to $dest"
            rc=1
        fi
    done

    # A manifest that does not declare the installs is a failure and must be
    # reported as one, not downgraded to a SKIP by the missing build tree below.
    [ "$rc" -eq 0 ] || return 1

    local tree="$BUILD_DIR/files"
    if [ ! -d "$tree" ]; then
        echo "no build tree at ${tree#"$ROOT"/}: the installed copies were NOT checked"
        echo "  run without --skip-flatpak to build one (stages 7-8)"
        # 98 = a skip the caller asked for, 99 = one a missing prerequisite
        # forced. Only the second costs a non-zero exit (D-31), and under
        # --skip-flatpak the absent tree is the flag's own consequence — the same
        # call the runner already makes for desktop-metainfo.
        [ "${SKIP_FLATPAK:-0}" -eq 1 ] && return 98
        return 99
    fi

    # Whether this tree was built from the manifest as it stands now cannot be
    # answered by mtime: flatpak-builder normalises timestamps in the exported
    # tree, so `files/` and everything under it are dated 1970 (checked). The
    # marker is the built binary instead — it appears only once the gamehandler
    # module's build-commands have run, which is the precondition for the
    # installs below to have run with them. No marker, no evidence: SKIP, and
    # say which half did run.
    if [ ! -f "$tree/bin/gamehandler" ]; then
        echo "no ${tree#"$ROOT"/}/bin/gamehandler, so this is not a completed build of the"
        echo "  gamehandler module — its installed copies were NOT checked"
        echo "  (the manifest half above did run, and passed)"
        return 99
    fi
    echo "build tree: ${tree#"$ROOT"/} (completed build of the gamehandler module)"

    local installed
    for entry in "${FLATPAK_CONTENTS[@]}"; do
        src="${entry%%|*}"
        dest="${entry#*|}"
        installed="$tree/$dest"
        if [ ! -f "$installed" ]; then
            echo "FAIL not in the built Flatpak: $dest"
            rc=1
        elif [ ! -s "$installed" ]; then
            echo "FAIL installed but empty: $dest"
            rc=1
        elif cmp -s "$ROOT/$src" "$installed"; then
            echo "ok   installed, byte-identical to $src: $dest ($(stat -c%s "$installed") bytes)"
        else
            echo "FAIL installed copy differs from $src: $dest"
            rc=1
        fi
    done
    return "$rc"
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
        # 99 and 77 are the stage saying "this did not run", and the run is
        # therefore incomplete: exit 3 (D-31). 98 is the stage saying "the caller
        # asked for this one" — a skip either way, but not evidence that the run
        # was truncated, so it stays a legitimate 0. A stage chooses between them
        # itself, because only the stage knows whether the missing thing was an
        # option or a toolchain.
        98) finish_skip "requested by the caller, not a missing prerequisite — see ${STAGE_LOG#"$ROOT"/}" 1 ;;
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

# Everything from here to `release_flatpak_lock` is one critical section over
# build-flatpak/ — stages 7-10, whether they build, run or merely read it. The
# trap is the backstop for the paths that exit from inside it (finish_fail
# exits when --keep-going is off); the explicit release afterwards hands the
# lock back before the summary so a waiter is not held while we print.
trap release_flatpak_lock EXIT
if ! acquire_flatpak_lock; then
    printf 'FAIL %s\n' "flatpak-lock"
    printf '     could not take %s — stages 7-10 were not run\n' "${FLATPAK_LOCK#"$ROOT"/}"
    FAILED+=("flatpak-lock")
    release_flatpak_lock
    summary
    exit 1
fi

if [ "$SKIP_FLATPAK" -eq 1 ]; then
    begin flatpak-build; finish_skip "--skip-flatpak" 1
elif require_tool flatpak-builder "from flatpak-builder"; then
    run_stage flatpak-build stage_flatpak
else
    begin flatpak-build; finish_skip "flatpak-builder is not installed"
fi

if [ "$SKIP_SMOKE" -eq 1 ]; then
    begin smoke-test; finish_skip "--skip-flatpak/--skip-smoke" 1
else
    run_stage smoke-test stage_smoke
fi

if [ "$SKIP_FLATPAK" -eq 1 ] && [ ! -d "$BUILD_DIR/files" ]; then
    begin desktop-metainfo
    # This one follows from --skip-flatpak, so it is a requested skip too: the
    # caller asked not to build, and is told the consequence is an unvalidated
    # installed copy rather than being failed for it.
    finish_skip "no build tree — run without --skip-flatpak to validate the installed copies" 1
else
    run_stage desktop-metainfo stage_desktop_metainfo
fi

# Runs even under --skip-flatpak: its manifest half needs no build tree, and that
# is precisely the half that catches a deleted install line. It reports SKIP on
# its own when there is no tree to inspect.
run_stage flatpak-contents  stage_flatpak_contents

# End of the build-flatpak/ critical section (see the lock note above).
release_flatpak_lock

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

# Exit codes, and why a skip is not always benign. A SKIP the caller asked for
# (--skip-flatpak / --skip-smoke) is a legitimate 0: the flag exists for exactly
# that, and a flag that skips work while still failing would be unusable in the
# fast local iteration loop it is documented for. A SKIP caused by a *missing
# prerequisite* is different in kind — the caller asked for a full verification
# and did not get one. Exiting 0 there is how "scripts/verify.sh passes from a
# clean checkout" (PLAN.md §9) becomes satisfiable by a run that never built or
# inspected the Flatpak: absent flatpak-builder, stage 7 skips, stage 10 loses
# its installed-copies half, and the whole thing reports success.
#
# A distinct code rather than 1, so a reader (and CI) can tell "a stage failed"
# from "the run was incomplete" — a defect to fix versus a toolchain to install.
# 3, not 2: 2 is already taken by the unknown-option usage error at the top of
# this script, and a CI step cannot tell `verify.sh --oops` from a partial run
# if both exit 2. The summary prints the same distinction, above the script's
# own line that already says a SKIP is not a pass; this is that sentence
# finally affecting the outcome.
#
# Note the lock's interaction: an unacquirable lock is a FAIL (1), not a skip,
# and the flock-absent branch in acquire_flatpak_lock is deliberately neither.
# Every stage still runs there, so the run is *complete* and stays a 0 — it is
# only un-serialised, and it says so in the output. Turning that into a 2 would
# report a missing util-linux as an incomplete verification, which is a false
# statement about stages that genuinely ran.
if [ "${#SKIPPED_UNREQUESTED[@]}" -gt 0 ]; then
    exit 3
fi
exit 0
