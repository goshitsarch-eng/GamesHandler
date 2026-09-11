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
# The stage list itself — names, order and one-line descriptions — is the STAGES
# array below, which `usage()` prints and `begin()` enforces. What follows is the
# reasoning behind each entry, which does not belong in an array element; if the
# two ever disagree about a *name*, STAGES is right and this is stale.
#
#   1  build                 cargo build (workspace root)
#   2  clippy                cargo clippy --all-targets -- -D warnings
#                            (workspace root ONLY — DECISIONS D-08)
#   3  test                  cargo test, with DISPLAY/WAYLAND_DISPLAY unset
#   4  cli                   the headless CLI. Drives `--list`/`--launch`/
#                            `--version` against a library this stage owns, and
#                            asserts the *non-empty* case — the empty one is
#                            what the smoke test already covered, and what a
#                            stub that never reads games.json passes (#31).
#   5  oracle-freshness      regenerate docs/migration/oracle/fixtures/ from the
#                            Python implementation and fail if the checked-in
#                            copy differs. This is what stops the Python
#                            contract from drifting silently.
#   6  python-tests          the existing Python suite must stay green
#                            (DECISIONS D-17: it is the behavioural reference
#                            the Rust port is written against)
#   7  cargo-sources         build-aux/flatpak/cargo-sources.json is fresh
#                            against Cargo.lock and covers every git source
#   8  flatpak-build         flatpak-builder builds the manifest
#   9  smoke-test            scripts/smoke-test.sh — CLI + headless GUI
#  10  desktop-metainfo      desktop-file-validate + appstreamcli validate
#  11  flatpak-contents      the built Flatpak carries the files no validator
#                            looks at: the application's own licence text, the
#                            Authenticode trust root, and the three metadata
#                            installs — which desktop-metainfo reads for
#                            well-formedness (two of them) but never for presence
#                            or path. Added by T-23, after the GPL text turned
#                            out to be missing from the Flatpak with every
#                            earlier stage green; widened to the metadata
#                            installs and to the manifest's whole install set by
#                            tasks #23/#24. It also compares the shipped binary
#                            against the source about pending pages (task #32),
#                            which is the one thing here that looks inside an
#                            executable rather than at a file it installs.
#
# The last four are one critical section under a `flock`: flatpak-build,
# smoke-test, desktop-metainfo and flatpak-contents all read or write
# build-flatpak/, and flatpak-build alone writes it destructively. The lock note
# above `acquire_flatpak_lock` explains why the lock spans the group rather than
# each stage; the short version is that a released lock in the middle is a window
# for another run to swap the tree, and the reader would then validate a
# different revision with every line still saying "ok".
#
# Those four are named rather than numbered on purpose. "stages 7-10" was a
# second copy of the stage order, kept in step with the list above by hand, and
# inserting the `cli` stage moved every number after it — the same drift the
# STAGES array exists to stop. Names do not move.
#
# The banner comment above each stage function is a name for the same reason,
# and it was a number for exactly as long as it took to insert `cli`: every
# banner from `oracle-freshness` down then said one less than it meant, and
# two of them both said "Stage 4". A name that is wrong is at least a name that
# disagrees with STAGES out loud.
#
# Output contract (packaging.md §6): one machine-greppable line per stage on
# stdout — `ok <stage>`, `FAIL <stage>`, `SKIP <stage>` — and every stage is
# preceded by `### <stage>`. Full tool output goes to
# target/verify-logs/<stage>.log; the head of that log is tail-able while a
# long stage runs, only the tail is echoed on failure, and a stage that reports
# its own sub-checks (the smoke test) has those echoed indented so a pass is
# still legible without opening the log.
#
# Read-only with respect to the repository. oracle-freshness regenerates the
# oracle into a temporary directory and compares, rather than writing over the
# tree, and asserts afterwards that the fixtures on disk are unchanged. Every
# other stage
# writes only to gitignored paths (target/, .flatpak-builder/, build-flatpak/,
# flatpak-repo/).
#
# Usage: scripts/verify.sh [options]
#
#   --skip-flatpak   skip the flatpak-build and smoke-test stages
#                    (fast local iteration)
#   --skip-smoke     skip the smoke-test stage only
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
# Stages the run never reached. `summary` fills this from STAGES minus ATTEMPTED;
# it is a global and not a local of `summary` because the exit tail has to act on
# it — see the note there. See also the assignment in `summary` for why an empty
# STOPPED is itself an error rather than a quiet pass.
UNRUN=()
# Split by cause, because only one of them is a legitimate exit 0: see the
# "Exit codes" note in the header. `finish_skip` fills these.
SKIPPED_REQUESTED=()
SKIPPED_UNREQUESTED=()
# Every stage that announced itself, in order — filled by `begin()`. Its length
# is also the cursor into STAGES: see the note above `begin`.
ATTEMPTED=()
# Why the run stopped early, for the summary's "did not run" line. Empty on a
# run that reached the end.
STOPPED=""

# name|function|what it is — THE stage list, in the order they run, and the only
# copy of it. Four things are derived from this array: the usage text, `begin()`'s
# refusal to announce a stage that is not listed here or is out of order, the
# summary's account of the stages an early exit never reached, and — since #48 —
# which function a stage runs.
#
# The function belongs here rather than at the `run_stage` call site for the
# reason #47 was fixed in `finish_ok` rather than in the exit tail: a pairing
# written in two places can disagree, and the run cannot tell. Eleven
# near-identical consecutive `run_stage` lines invite a copy-paste slip, and
# `run_stage cli stage_test` reported `cli` as passed while the test suite ran in
# its place — the summary byte-identical to a correct run (#48). With the pairing
# in the table, `run_stage` takes only a name and there is no second argument to
# get wrong: the wrong state cannot be written, rather than being detected after
# it has been.
#
# It cannot be derived by convention. `oracle-freshness -> stage_oracle`,
# `flatpak-build -> stage_flatpak` and `smoke-test -> stage_smoke` all fail
# `stage_${name//-/_}`, so a table is needed either way — and needing one either
# way is the argument for it being the only one.
#
# What this does **not** catch, since a table cannot check itself against the
# code it names: an entry whose function is a different *existing* stage
# function — `"cli|stage_test|..."`. That is measured, not assumed: it exits 0
# and reports `cli` passed with the test suite's output, exactly as before. The
# removal closes the realistic trigger (a slip across eleven near-identical
# call-site lines, which is no longer expressible) and leaves the deliberate
# edit, which is the same residual any source file has: no structure prevents
# someone from writing the wrong thing on purpose. It is stated here rather than
# left implied, because claiming more than that is how the comment this fix
# replaced came to be false.
#
# That last one is why it exists. The order used to be implied by the sequence of
# `run_stage` calls and restated in the usage text, and nothing compared them, so
# a run that died at oracle-freshness printed `skipped: none` while six stages had
# not been attempted at all — and "failed: oracle-freshness" on its own reads as
# "only the oracle is broken" (task #29; the lead read it that way himself).
STAGES=(
    "build|stage_build|the workspace builds"
    "clippy|stage_clippy|cargo clippy --all-targets -- -D warnings (workspace root ONLY, D-08)"
    "test|stage_test|the test suite, in BOTH feature configurations (task #27)"
    "cli|stage_cli|the headless CLI --list/--launch/--version, against a library it must read"
    "oracle-freshness|stage_oracle|the checked-in fixtures equal what the Python generators produce"
    "python-tests|stage_python|the Python suite stays green (D-17)"
    "cargo-sources|stage_cargo_sources|cargo-sources.json is fresh against Cargo.lock and covers every git source"
    "flatpak-build|stage_flatpak|flatpak-builder builds the manifest"
    "smoke-test|stage_smoke|scripts/smoke-test.sh — CLI + headless GUI"
    "desktop-metainfo|stage_desktop_metainfo|desktop-file-validate + appstreamcli validate"
    "flatpak-contents|stage_flatpak_contents|the installed Flatpak files, and the tree its binary came from"
)

# The banner comment above each stage function names the stage, and this asserts
# it names the *right* one. Until `cli` was inserted those banners were
# numbered, and nothing compared them with STAGES: every banner from
# `oracle-freshness` down then said one less than it meant, and two of them both
# said "Stage 4". That is task #22's defect class one layer out — an assertion in
# a comment that stopped being true, read by everyone and checked by nobody —
# and the cost of catching it by eye is that you have to already suspect it.
#
# Names do not drift the way numbers do, and this makes even a wrong name a hard
# error rather than a plausible-looking one. Exit 2, like `begin()`'s
# out-of-order refusal: it is a bug in this script, not a property of the tree,
# so it is not a stage result and must not be reported as one.
#
# What it does NOT check, so nobody over-trusts it: this compares the *sequence*
# of banners with the *sequence* of stages, not which function each banner sits
# above. Move a banner to another function without changing its position in the
# file and this stays green. Attachment is not expressible in a comment stream,
# and a comment is not a thing a language can be made to check — which is the
# same limit that let a stale claim sit three lines above the `cli` stage while
# every structural check here passed. Treat a green banner_check as "the names
# and their order agree", which is what it says, and nothing more.
banner_check() {
    local -a declared=() banners=()
    local entry
    for entry in "${STAGES[@]}"; do
        declared+=("${entry%%|*}")
    done
    mapfile -t banners < <(sed -n 's/^# Stage: \([^ ]*\).*/\1/p' "${BASH_SOURCE[0]}")
    if [ "${declared[*]}" != "${banners[*]}" ]; then
        printf 'verify.sh: the "# Stage:" banners disagree with STAGES\n' >&2
        printf '  STAGES:  %s\n' "${declared[*]}" >&2
        printf '  banners: %s\n' "${banners[*]}" >&2
        printf '  both are in %s, in the order they must appear\n' "${BASH_SOURCE[0]}" >&2
        exit 2
    fi
}
banner_check

usage() {
    cat <<'EOF'
usage: scripts/verify.sh [options]

  --skip-flatpak   skip the flatpak-build and smoke-test stages
                   (fast local iteration)
  --skip-smoke     skip the smoke-test stage only
  --keep-going     run every stage and report the full table instead of
                   stopping at the first failure
  --offline        pass --disable-download to flatpak-builder
  --hold SECONDS   forward the GUI hold interval to the smoke test

Stages, in order:
EOF
    local entry
    for entry in "${STAGES[@]}"; do
        # `##*|`, not `#*|`: the entries carry the stage's function as a middle
        # field, so the first `|` is no longer where the description starts.
        printf '  %-18s %s\n' "${entry%%|*}" "${entry##*|}"
    done
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
# The stage function `run_stage` actually invoked for the current stage. Empty
# between `begin` and that call, so `finish_ok` can tell "this stage ran and
# returned 0" from "someone announced this stage and then claimed a result" —
# see the note in `finish_ok` for what it is there to catch.
STAGE_RAN=""

# Announce a stage. Every stage goes through here — the `run_stage` calls and the
# direct `begin <stage>; finish_skip ...` ones alike — which is what makes this
# the right place to enforce STAGES rather than merely read it.
#
# A stage not in STAGES would be invisible to `summary` (it is neither passed,
# failed nor skipped) and so could vanish from the report exactly as the six
# unattempted stages did in task #29; a stage out of order means the array and
# the run disagree about what runs when. Both are a bug in this script, not a
# stage result, so both are a usage error (2) rather than a FAIL — and they are
# caught on the run that introduces them rather than on the next reader's.
begin() {
    STAGE="$1"
    local index="${#ATTEMPTED[@]}" declared="${STAGES[${#ATTEMPTED[@]}]:-}"
    declared="${declared%%|*}"
    if [ -z "${STAGES[${#ATTEMPTED[@]}]:-}" ] || [ "$STAGE" != "$declared" ]; then
        printf 'verify.sh: begin %s — STAGES says stage %d is %s\n' \
            "$STAGE" "$index" "${declared:-<past the end of the list>}" >&2
        printf '  the STAGES array is the order the stages must run in; add the\n'
        printf '  stage there, or move this call to where it belongs.\n' >&2
        exit 2
    fi
    ATTEMPTED+=("$STAGE")
    # Cleared here and set only by `run_stage`, so the marker always describes
    # *this* stage and never survives into the next one.
    STAGE_RAN=""
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
    # A stage reports `ok` only if `run_stage` ran its function for it. Without
    # this, `begin cli; finish_ok` — a stage that announces itself and then
    # claims a pass — was reported as passed and the run exited 0, because
    # `begin` had already appended the stage to ATTEMPTED and the unrun check in
    # the exit tail therefore could not see it. That is a false claim about
    # coverage, made by the file whose job is deciding whether the project is
    # covered, and it is the same class of defect as the rest of this runner:
    # a result that was never produced.
    #
    # This is a bug in verify.sh rather than a stage outcome, so it is a usage
    # error (2) like `begin`'s STAGES guard and `run_stage`'s `declare -F` one,
    # and for the same reason: it should stop the run that introduces it rather
    # than be reported as a failing stage and send a reader to the code under
    # test. Nothing is printed first — an `ok` line here would be the very lie
    # this refuses to tell.
    if [ -z "$STAGE_RAN" ]; then
        printf 'verify.sh: %s reported ok, but no stage function ran for it\n' "$STAGE" >&2
        printf '  `ok` is claimed by `finish_ok`, which reads the exit status\n' >&2
        printf '  `run_stage` captured from the stage function. A bare\n' >&2
        printf '  `begin %s; finish_ok` produces no such status.\n' "$STAGE" >&2
        printf '  call the stage through `run_stage`, or skip it with finish_skip.\n' >&2
        exit 2
    fi
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
    # Stages that never announced themselves. They are not skips — nothing was
    # decided about them, they were never reached — and leaving them out of the
    # report is how a run that died there came to read as "only the oracle
    # is broken" while six stages had not run (task #29). They cannot appear in
    # PASSED/FAILED/SKIPPED, because none of those is ever set for a stage that
    # did not begin.
    local i
    for ((i = ${#ATTEMPTED[@]}; i < ${#STAGES[@]}; i++)); do
        UNRUN+=("${STAGES[i]%%|*}")
    done
    if [ "${#UNRUN[@]}" -gt 0 ]; then
        # No STOPPED means the run *finished* without reaching them, which can
        # only be STAGES and the run disagreeing — not an abort.
        printf 'did not run (%s): %s\n' \
            "${STOPPED:-the run finished without reaching them}" "${UNRUN[*]}"
        printf '  these are neither passes nor skips: nothing was verified about them\n'
    fi
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
        STOPPED="an earlier stage failed"
        summary
        exit 1
    fi
}

# ---------------------------------------------------------------------------
# The Flatpak build lock
#
# The four locked stages all touch build-flatpak/: flatpak-build writes it
# destructively (--force-clean erases the tree), smoke-test runs the app out of
# it, and desktop-metainfo and flatpak-contents read the installed copies from
# it. Two verify.sh runs in one checkout therefore corrupt each other — one wipes the tree while the other is reading it, which makes
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
        # hard here would take the earlier stages down with it on a box without
        # util-linux, which would be a worse trade — those stages are unaffected
        # by this hazard.
        echo "flock is not installed, so the build-flatpak/ stages are NOT serialised"
        echo "  against another verify.sh in this checkout. If none is running, the"
        echo "  result"
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
# Stage: build
# ---------------------------------------------------------------------------
stage_build() {
    cargo build
}

# ---------------------------------------------------------------------------
# Stage: clippy — at our workspace root only (DECISIONS D-08)
# ---------------------------------------------------------------------------
stage_clippy() {
    cargo clippy --all-targets -- -D warnings
}

# ---------------------------------------------------------------------------
# Stage: test — with no display, to prove the logic suite is headless
#
# TWO configurations, because they are not the same gate. `cargo test` with no
# -p unifies features across every workspace member; `-p gamehandler-core` builds
# that crate's own graph, and cosmic-theme turns on `serde_json/preserve_order`,
# which is inherited by anything that depends on libcosmic. Measured (D-33):
#
#   cargo tree -p gamehandler-core -e features -i serde_json  -> no preserve_order
#   cargo tree -p gamehandler      -e features -i serde_json  -> preserve_order
#   cargo tree                     -e features -i serde_json  -> preserve_order
#         via serde_json feature "preserve_order" <- cosmic-theme <- libcosmic
#
# So a `serde_json::Map` is a BTreeMap (key order: alphabetical) in the first
# configuration and insertion-ordered in the other two — and the two disagree in
# practice: a vector case passed narrow and failed wide. Running only the wide
# one, which is what this stage did until #27, leaves the narrow configuration
# ungated, so a change that depends on it passes here and fails for anyone
# building that crate alone. The app crate needs no separate run: it depends on
# libcosmic, so it is the wide configuration already.
#
# D-33 records the mechanism and the rule that follows from it: a verdict
# without its build scope is not evidence. This stage runs both scopes so the
# verdict does not have one.
#
# The narrow run gets its own CARGO_TARGET_DIR. Features are part of cargo's
# fingerprint, so flipping between the two configurations in one target
# directory rebuilds core's whole dependency graph on every alternation — twice
# per verify run, for nothing. It is a subdirectory of *this* checkout's
# gitignored target/, not a shared or borrowed cache: an artifact built from
# another source tree landing in target/ is a real hazard here (it cost the lead
# an hour), and this is the one thing that avoids rather than courts it.
#
# Production serialisation is unaffected either way, so this is not a shipping
# risk: json::to_python_string calls value.serialize() on the typed value, so no
# serde_json::Map is ever constructed and Library::save / Settings::save write
# identical bytes in both configurations. The exposure is test-side `to_value`
# only — which is precisely why it is worth a gate rather than an argument.
# ---------------------------------------------------------------------------
stage_test() {
    env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
        cargo test || return 1
    env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
        CARGO_TARGET_DIR="$ROOT/target/verify-narrow" \
        cargo test -p gamehandler-core
}

# ---------------------------------------------------------------------------
# Stage: cli — the headless CLI, against a library it has to actually read
#
# `--list`, `--launch` and `--version` are what the app's own desktop shortcuts
# invoke, and D-12 makes them a hard requirement: no display, no GPU, no
# toolkit. The smoke test covers all three, but only half of each — it asserts
# `--list` *exits 0* and never looks at what was printed, and it runs against
# whatever library the ambient environment happens to point at. A `--list` that
# printed the empty-library line unconditionally therefore passed it, and did:
# `--list` was a stub that never opened games.json while a real library sat in
# `$XDG_CONFIG_HOME` (#31, task #31's gate). The lesson generalises, and is why
# this stage exists rather than a fourth line in the smoke test: **a check that
# only exercises the empty case passes on a program that never reads the file.**
#
# So this stage owns its library and asserts the non-empty case. It uses the
# binary the build stage just built — the artifact `cargo test` tested — not the
# Flatpak's release binary, so it needs no build tree and no lock; the Flatpak's
# own copy is the smoke test's business.
#
# Two things about the assertions, both deliberate:
#
#   * the non-empty check compares *bytes* against the expected two rows, so
#     extra output, a missing row, a missing trailing newline and the wrong
#     order all fail. The fixture's names ("apple", "Banana") are a sorting
#     trap: the contract is `Library::all()`'s default sort by the *lowercased*
#     name (gamehandler/models.py:156-163), so "apple" precedes "Banana", where
#     a byte-wise sort would reverse them;
#   * `--launch` is asserted on the **exit code**, with the stderr reason
#     required as well. Asserting only the text would pass on a program that
#     prints the right words and returns the wrong status — which is exactly
#     what the stub does. A *known* id is not launched: that would start a real
#     game through Wine, which is not something a verification gate should do.
#
# Hermetic by construction: HOME and all three XDG bases point into one temp
# dir, so nothing the binary writes can reach the developer's library or the
# checkout, and nothing it reads can come from the ambient environment.
#
# It was written while #31 was still open, and the note that stood here — "this
# stage is RED until #31 lands, that is intended" — was never true: `e9ecf5e`
# fixed #31 before the commit that added this text, and the stage was green from
# its first run. It is replaced rather than deleted, because a note like that
# does not only misdescribe the past, it instructs the future: it tells whoever
# next sees this stage go red that the red is expected, three lines above the
# code that exists to catch exactly that regression. **A red cli stage is a
# defect.** The one exception is a missing binary, which the first branch below
# reports as such and which the build stage would have failed on already.
# ---------------------------------------------------------------------------
stage_cli() {
    local bin="$ROOT/target/debug/gamehandler"
    if [ ! -x "$bin" ]; then
        echo "no binary at ${bin#"$ROOT"/}, which is what the build stage produces"
        return 1
    fi

    local tmp
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/gh-cli-XXXXXX")" || return 1
    # Two libraries: one a user has never written (no games.json at all), one
    # with two games.
    #
    # `fresh` and `two` are the *XDG_CONFIG_HOME values*, not the directories
    # holding games.json: the app appends `gamehandler/games.json` to whatever
    # base it is given (`paths.rs:56-65` → `base_dir(...).join("gamehandler")`),
    # so passing the inner directory here would point the binary one level too
    # deep and every check would see an empty library. It did, while this stage
    # was being written — and the *empty* check could not tell the difference,
    # which is the same asymmetry that let #31 through in the first place.
    local fresh="$tmp/fresh" two="$tmp/two"
    mkdir -p "$fresh/gamehandler" "$two/gamehandler" || { rm -rf "$tmp"; return 1; }
    cat >"$two/gamehandler/games.json" <<'JSON'
[{"id": "11111111111111111111111111111111", "name": "apple"},
 {"id": "22222222222222222222222222222222", "name": "Banana"}]
JSON

    local out="$tmp/out" err="$tmp/err" expected="$tmp/expected"
    local rc=0 failures=0

    # cli_run <config-home> <args...> — the binary, with no display and with
    # every base directory inside $tmp.
    #
    # Defined here rather than with the other helpers at the top because it is
    # inseparable from `tmp`: a top-level definition would have to take the
    # directory as an argument and would then be callable from a stage that has
    # no temp dir of its own, which is a chance to get it wrong for no gain.
    cli_run() {
        local config="$1"; shift
        env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
            HOME="$tmp" \
            XDG_CONFIG_HOME="$config" \
            XDG_DATA_HOME="$tmp/data" \
            XDG_CACHE_HOME="$tmp/cache" \
            "$bin" "$@"
    }

    # --- cli-version ---
    rc=0; cli_run "$fresh" --version >"$out" 2>"$err" || rc=$?
    if [ "$rc" -eq 0 ] && grep -qE '^GameHandler [0-9]+\.[0-9]+\.[0-9]+$' "$out"; then
        echo "ok   cli-version ($(head -n1 "$out"))"
    else
        echo "FAIL cli-version: expected exit 0 and 'GameHandler <x.y.z>', got exit $rc"
        sed 's/^/     | /' "$out" "$err"
        failures=$((failures + 1))
    fi

    # --- cli-list-empty ---
    #
    # The library file is absent, which is what a fresh install looks like. The
    # app's name is NOT restated here: it is `gamehandler_core::APP_NAME`, and a
    # check that pastes the constant it is checking is the defect D-28 was
    # written about. The message's shape is pinned instead.
    rc=0; cli_run "$fresh" --list >"$out" 2>"$err" || rc=$?
    if [ "$rc" -eq 0 ] && [ "$(wc -l <"$out")" -eq 1 ] \
        && grep -qE '^[^:]+: the library is empty$' "$out"; then
        echo "ok   cli-list-empty (exit 0, the empty-library line)"
    else
        echo "FAIL cli-list-empty: expected exit 0 and exactly one empty-library line, got exit $rc"
        sed 's/^/     | /' "$out" "$err"
        failures=$((failures + 1))
    fi

    # --- cli-list-non-empty ---
    #
    # The check #31 slipped past. Byte-for-byte, both rows, in order.
    printf '11111111111111111111111111111111\tapple\n22222222222222222222222222222222\tBanana\n' >"$expected"
    rc=0; cli_run "$two" --list >"$out" 2>"$err" || rc=$?
    if [ "$rc" -eq 0 ] && cmp -s "$expected" "$out"; then
        echo "ok   cli-list-non-empty (exit 0, 2 rows id<TAB>name, name-sorted)"
    else
        echo "FAIL cli-list-non-empty: a 2-game library at \$XDG_CONFIG_HOME/gamehandler/games.json"
        echo "     must print its two rows; got exit $rc and:"
        echo "     --- expected ---"; sed 's/^/     | /' "$expected"
        echo "     --- stdout ---";   sed 's/^/     | /' "$out"
        if [ -s "$err" ]; then echo "     --- stderr ---"; sed 's/^/     | /' "$err"; fi
        failures=$((failures + 1))
    fi

    # --- cli-launch-unknown ---
    rc=0; cli_run "$two" --launch this-id-is-not-in-the-library >"$out" 2>"$err" || rc=$?
    if [ "$rc" -ne 0 ] && grep -q "no game with id" "$err"; then
        echo "ok   cli-launch-unknown (exit $rc, reason on stderr)"
    else
        echo "FAIL cli-launch-unknown: an unknown id must exit non-zero with the reason on"
        echo "     stderr; got exit $rc, stderr: $(head -n1 "$err")"
        failures=$((failures + 1))
    fi

    rm -rf "$tmp"
    [ "$failures" -eq 0 ] || return 1
    return 0
}

# ---------------------------------------------------------------------------
# Stage: oracle-freshness — the fixtures on disk equal what the generators produce
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
    # cargo-sources generator), the app, or the network.
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
# Stage: python-tests — the Python suite stays green (DECISIONS D-17)
# ---------------------------------------------------------------------------
stage_python() {
    env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
        python3 -m unittest discover -s tests -t . -v
}

# ---------------------------------------------------------------------------
# Stage: cargo-sources — cargo-sources.json freshness + git coverage
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
# Stage: flatpak-build — flatpak-builder builds the manifest
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
# Stage: smoke-test — headless smoke test (scripts/smoke-test.sh)
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
# Stage: desktop-metainfo — desktop entry and AppStream metadata
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
# Stage: flatpak-contents — the installed files, and the tree the binary came from
#
# The desktop-metainfo stage loads the desktop entry and the metainfo and asks
# whether they are *well-formed*. Nothing before this stage asks whether the
# Flatpak contains the application's own licence, the Authenticode trust root,
# or the icon at all — none of those files is an input to a validator. That gap
# is not hypothetical: T-16 moved the build from meson (which ran
# data/meson.build's install_data) to cargo, dropped the licence install with
# it, and every stage stayed green while the Flatpak shipped a GPL-3 binary with
# no licence. T-23.
#
# The list is every file the gamehandler module installs *except the binary*,
# and what covers each one before this stage gets to it:
#
#   LICENSE, the trust root   nothing at all.
#   desktop entry, metainfo   desktop-metainfo reads their *content* for
#                             well-formedness (on the built copy when there is
#                             one). Nothing read
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
# 9." That is off by one — that stage loads two of the three, and never their
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
# destination that is neither in the list nor the binary (which the
# flatpak-build and smoke-test stages exercise by running it). Called with the destinations as arguments, so the list
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
        echo "  run without --skip-flatpak to build one (the flatpak-build stage)"
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

    # --- the binary is the tree that was tested (task #32) ------------------
    #
    # Nothing above looks inside the binary, and nothing in this script did:
    # every stage compares files, and a Flatpak whose six page bodies are all
    # `pending_page` — the entire user-visible application — was
    # indistinguishable from a finished one to all of them. That is task #32,
    # and the reason the repository could not tell scaffold from shipped.
    #
    # `crates/app/tests/pending_pages.rs` is the primary guard: it pins the SET
    # of pages still rendering the placeholder, so landing a page without
    # editing the pin fails and names the page. This is the half that guard
    # cannot make on its own, because it reads the *source*. A Flatpak built
    # from a different tree than the one this run tested is outside its reach,
    # and that is the scope the tag ships.
    #
    # `pending_page` is the single generator of every placeholder body, so its
    # text is a reliable fingerprint of "this binary still has placeholder
    # pages". The assertion is that the fingerprint AGREES with the source:
    # either both have placeholders or neither does.
    #
    # WHERE THIS CAN ACTUALLY FIRE, since the honest answer is narrower than it
    # first looks. Under a FULL run it cannot: `flatpak-build` ran a few stages
    # earlier in this same process, under this same lock, with `--force-clean`,
    # which erases build-flatpak/ before building — so the tree here was built
    # from the source as it stands, and the two sides always agree. On that path
    # this branch is unreachable, and it is not a defence against a stale build
    # tree.
    #
    # The reachable path is a run that did NOT build: `--skip-flatpak` or
    # `--skip-smoke` skips `flatpak-build` and still reaches here, against
    # whatever build-flatpak/ was left by an earlier run. And porting a page is
    # precisely the change the rest of this stage cannot see — it edits
    # `crates/app/src/main.rs`, which is in none of the five FLATPAK_CONTENTS
    # entries, so the byte-identity checks stay green while the tree still ships
    # six placeholders. Without this check, `verify.sh --skip-flatpak` after a
    # page lands reports a clean Flatpak that is a stub. That is the branch's
    # real content, and the flagged case below was produced that way: the build
    # tree left by an earlier full run, and a source in which all six pages had
    # landed — the all-placeholder Flatpak of #32, one difference of sign away.
    #
    # Note the bound on the claim: it does not check that the *same* pages are
    # placeholders on both sides, only that the two agree about whether any are.
    # Pinning the set is `crates/app/tests/pending_pages.rs`'s job; this is the
    # artifact-side cross-check, and the two are reported separately so neither
    # can be read as the other.
    #
    # `-a` because the binary is not text. The counts are reported rather than
    # asserted against a fixed number, because how many times an ELF keeps a
    # shared literal is not something this script can claim to know.
    #
    # Both sides are counted the SAME way — `grep -o` piped to `wc -l`, i.e.
    # occurrences — and that is deliberate. `grep -c` counts *matching lines*,
    # which is a different number the moment two arms share a line, and the two
    # halves would then be comparing a line count against an occurrence count
    # and disagreeing for no reason. The source side is anchored to a match arm
    # (`Page::X => pending_page(Page::` at the start of a line), because an
    # unanchored search counts a comment that quotes the call shape: measured,
    # one such comment takes the source count from 5 to 6 and makes the two
    # halves agree for the wrong reason. Anchoring does not make this a parse of
    # the Rust — a comment inside the match that begins with an arm-like prefix
    # still counts — which is why the pass line says what it checked rather than
    # claiming it counted call sites. The parse is the test's job.
    local in_source in_binary
    in_source="$(grep -oE '^[[:space:]]*Page::[A-Za-z]+ => pending_page\(Page::' \
        "$ROOT/crates/app/src/main.rs" | wc -l | tr -d '[:space:]')"
    in_binary="$(grep -ao -- 'This page has not been ported yet (' \
        "$tree/bin/gamehandler" | wc -l | tr -d '[:space:]')"
    if [ "$in_source" -gt 0 ] && [ "$in_binary" -gt 0 ]; then
        echo "ok   the shipped binary carries the placeholder text, as the source does"
        echo "     ($in_source arm(s) dispatch to pending_page; $in_binary occurrence(s) in the ELF)"
        echo "     both sides are non-zero, so this checked that the two AGREE that"
        echo "     placeholder pages exist — NOT which pages they are. The check that"
        echo "     names pages is crates/app/tests/pending_pages.rs, which is reported"
        echo "     by the test stage; this half is only the artifact cross-check."
    elif [ "$in_source" -eq 0 ] && [ "$in_binary" -eq 0 ]; then
        echo "ok   every page is ported in the source, and the shipped binary has no placeholder"
        echo "     neither side mentions the placeholder, which is T-19's end state; this"
        echo "     says nothing about whether the six pages render anything correct"
    else
        echo "FAIL the shipped binary and the source disagree about the placeholder pages"
        echo "     crates/app/src/main.rs: $in_source arm(s) dispatching to pending_page"
        echo "     ${tree#"$ROOT"/}/bin/gamehandler: $in_binary occurrence(s) of the placeholder text"
        echo "     so this Flatpak was not built from the source this run was given."
        echo "     If this run built it (no --skip-flatpak), that is a contradiction and"
        echo "     worth chasing — --force-clean should have made it impossible."
        echo "     If it did not (--skip-flatpak, or a tree left by a build.sh run), then"
        echo "     build-flatpak/ predates the edit and the five installed files above"
        echo "     cannot show it: none of them is crates/app/src/main.rs. Rebuild."
        rc=1
    fi

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

# Every `stage_*` function the script defines is named by exactly one STAGES
# entry, and every entry names a defined function.
#
# Since #48 the table is the only place the stage/function pairing is written,
# which makes it the single point of failure: an entry edited to name another
# *existing* stage function reports the wrong stage as passed, and nothing in
# the run can tell (measured — it exits 0 with `ok cli` while the test suite's
# log is the one written). This does not check the *mapping*, which is a thing a
# table cannot check against the code it names. It checks the **bijection**,
# which is enough for the case that has to be closed: an entry typo strands its
# own function (set `cli|stage_test` and `stage_cli` is named by nothing) and, if
# it points at another stage's function, names that one twice. The report names
# all three kinds — stranded, duplicated, dangling — because which one you get
# depends on which way the edit went.
#
# What still slips through, stated so this is not read as covering more: a
# permutation that preserves the bijection — `cli -> stage_test` *and*
# `test -> stage_cli` together — still runs the wrong functions under the right
# names. That is two coordinated edits, not one slip, and it is a far smaller
# target than the eleven near-identical call-site lines #48 closed.
#
# Called from the runner rather than from the top of the file with the other
# guards, because bash executes top-down: at the top the stage functions are not
# defined yet, so this would find none and pass vacuously. That is the same trap
# as a test that asserts on an empty collection.
check_stage_table() {
    local -a named=() defined=()
    local entry fn problem=""

    for entry in "${STAGES[@]}"; do
        fn="${entry#*|}"; fn="${fn%%|*}"
        named+=("$fn")
    done
    # `declare -F` prints "declare -f <name>" per line, so $3 is the name.
    mapfile -t defined < <(declare -F | awk '{print $3}' | grep '^stage_' | sort)

    # A defined stage function that no entry names — the entry-typo case.
    for fn in "${defined[@]}"; do
        if ! printf '%s\n' "${named[@]}" | grep -qx -- "$fn"; then
            problem+="  ${fn} is defined but no STAGES entry names it\n"
        fi
    done
    # A function named by more than one entry, and an entry naming something
    # that is not defined. The second is also caught by `run_stage`, but only
    # once that stage is reached — after other stages have already run.
    for fn in "${named[@]}"; do
        if ! declare -F "$fn" >/dev/null; then
            problem+="  STAGES names ${fn}, which is not a function in this script\n"
        fi
    done
    for fn in $(printf '%s\n' "${named[@]}" | sort | uniq -d); do
        problem+="  ${fn} is named by more than one STAGES entry\n"
    done

    if [ -n "$problem" ]; then
        printf 'verify.sh: the STAGES table and the script disagree about the stage functions\n' >&2
        printf '%b' "$problem" >&2
        printf '  every `stage_*` function the script defines must be named by exactly\n' >&2
        printf '  one STAGES entry, and every entry must name one — see the note on the\n' >&2
        printf '  table above, which is the only place the pairing is written.\n' >&2
        exit 2
    fi
}

check_stage_table

run_stage() {
    # One argument, because the function comes from STAGES. See the note above
    # the array: the pairing used to be written here as well, and a copy-paste
    # slip across eleven near-identical lines ran the wrong stage under the right
    # name while the run reported success (#48).
    local name="$1"
    # The same cursor `begin` uses, read *before* `begin` appends to ATTEMPTED,
    # so this is the entry for the stage about to start.
    local entry="${STAGES[${#ATTEMPTED[@]}]:-}"
    local fn="${entry#*|}"; fn="${fn%%|*}"
    if [ -z "${entry:-}" ]; then
        printf 'verify.sh: run_stage %s — STAGES has no stage %d to take a function from\n' \
            "$name" "${#ATTEMPTED[@]}" >&2
        exit 2
    fi
    # A typo in the table would otherwise surface as "command not found" (127)
    # and be reported as a failing stage, which sends the reader looking at the
    # code under test rather than at the STAGES entry.
    if ! declare -F "$fn" >/dev/null; then
        printf 'verify.sh: STAGES entry %s names %s, which is not a function in this script\n' \
            "$name" "$fn" >&2
        exit 2
    fi
    begin "$name"
    local rc=0
    # Set immediately before the call and after `begin` has cleared it, so the
    # window in which `finish_ok` accepts a stage is exactly the window in which
    # its function has run.
    STAGE_RAN="$fn"
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

run_stage build
run_stage clippy
run_stage test
run_stage cli
run_stage oracle-freshness
run_stage python-tests
run_stage cargo-sources
# Everything from here to `release_flatpak_lock` is one critical section over
# build-flatpak/ — the four locked stages, whether they build, run or merely
# read it. The
# trap is the backstop for the paths that exit from inside it (finish_fail
# exits when --keep-going is off); the explicit release afterwards hands the
# lock back before the summary so a waiter is not held while we print.
trap release_flatpak_lock EXIT
if ! acquire_flatpak_lock; then
    printf 'FAIL %s\n' "flatpak-lock"
    printf '     could not take %s — the build-flatpak/ stages were not run\n' "${FLATPAK_LOCK#"$ROOT"/}"
    FAILED+=("flatpak-lock")
    release_flatpak_lock
    STOPPED="the Flatpak build lock could not be taken"
    summary
    exit 1
fi

if [ "$SKIP_FLATPAK" -eq 1 ]; then
    begin flatpak-build; finish_skip "--skip-flatpak" 1
elif require_tool flatpak-builder "from flatpak-builder"; then
    run_stage flatpak-build
else
    begin flatpak-build; finish_skip "flatpak-builder is not installed"
fi

if [ "$SKIP_SMOKE" -eq 1 ]; then
    begin smoke-test; finish_skip "--skip-flatpak/--skip-smoke" 1
else
    run_stage smoke-test
fi

if [ "$SKIP_FLATPAK" -eq 1 ] && [ ! -d "$BUILD_DIR/files" ]; then
    begin desktop-metainfo
    # This one follows from --skip-flatpak, so it is a requested skip too: the
    # caller asked not to build, and is told the consequence is an unvalidated
    # installed copy rather than being failed for it.
    finish_skip "no build tree — run without --skip-flatpak to validate the installed copies" 1
else
    run_stage desktop-metainfo
fi

# Runs even under --skip-flatpak: its manifest half needs no build tree, and that
# is precisely the half that catches a deleted install line. It reports SKIP on
# its own when there is no tree to inspect.
run_stage flatpak-contents
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
# inspected the Flatpak: absent flatpak-builder, flatpak-build skips,
# flatpak-contents loses
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
# Unrun stages are the same defect as an unrequested skip, and reaching them is
# easier: adding a stage means editing STAGES, adding a banner and writing the
# function — and forgetting the `run_stage` line. That is a stage that never
# announces itself, so it is in neither PASSED nor FAILED nor SKIPPED, and
# without this branch the run reports every stage green and exits 0 while a
# stage it advertises never ran.
#
# This branch sees only the stages that never announced themselves — it reads
# ATTEMPTED, so a stage that called `begin` and then claimed a result *without
# running* is invisible to it. That case is not hypothetical: `begin cli;
# finish_ok` used to report `cli` as passed and the run as 0 (finding #47,
# pre-existing — it reproduces at 50798f9^ as well as at 50798f9, and the #41
# fix neither introduced it nor was meant to cover it). It is refused earlier
# now, in `finish_ok`, which is where the claim is made; this note is here so
# that the next reader of this branch does not assume it is the backstop for
# every way a stage can claim a result it did not produce. It is one of them.
#
# A `run_stage` naming a function that does not exist is caught too, but not
# here — `run_stage`'s own `declare -F` guard exits 2 hundreds of lines above,
# before the stage is even announced.
#
# It is the sentence below, applied to the other way a run can be partial. That
# one is about a *skip*; this is about never getting there at all, and a reader
# of "passed: build ... flatpak-contents" cannot tell the two apart.
if [ "${#SKIPPED_UNREQUESTED[@]}" -gt 0 ] || [ "${#UNRUN[@]}" -gt 0 ]; then
    if [ "${#UNRUN[@]}" -gt 0 ] && [ -z "$STOPPED" ]; then
        printf '\nNOTE: %s never ran, and nothing stopped the run reaching them.\n' "${UNRUN[*]}"
        printf 'The stage list and the runner disagree — an unrun stage is not a pass.\n'
    fi
    exit 3
fi
exit 0
