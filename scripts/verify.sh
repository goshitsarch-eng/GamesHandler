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
#   --stage-timeout SECONDS
#                    ceiling on a single stage; 0 disables the deadline. The
#                    default is 900, with per-stage overrides in STAGE_TIMEOUTS
#                    (`flatpak-build` gets an hour). A stage that overruns is
#                    killed, reported as a FAIL naming the stage, and — like any
#                    other failure — stops the run unless --keep-going (#103).
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
    "doc|stage_doc|rustdoc resolves every intra-doc link (-D rustdoc::broken_intra_doc_links, task #75)"
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
  --stage-timeout SECONDS
                   ceiling on a single stage; 0 disables it. Default 900,
                   with per-stage overrides for the stages that go long
                   (flatpak-build: 3600). A stage that overruns is killed
                   and reported as a FAIL naming the stage.

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
        --stage-timeout) STAGE_TIMEOUT="${2:?--stage-timeout needs a value}"; shift 2 ;;
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
# Non-empty while the stage being reported was killed by its own ceiling, so
# `finish_fail` says "timed out" rather than leaving the reader to guess from an
# empty log whether the stage hung or failed. Cleared by `begin`.
STAGE_TIMED_OUT=""

# --- the stage deadline (#103) --------------------------------------------
#
# `grep -n timeout scripts/verify.sh` returned nothing, so every stage ran with
# no deadline at all: a stage that never returned wedged the run, and a wedged
# run emits no output to misread — only a command that never comes back. The
# case that filed this was real and was observed: two orphaned
# `netpaths::tests::unquote_matches_cpython` binaries spun at 99.1% CPU for six
# hours, and the stage waiting on them would have waited forever.
#
# The ceiling is generous on purpose — a deadline that a real stage can hit is
# worse than none, because it turns a slow build into a red run and teaches the
# reader to distrust the gate. `flatpak-build` is the longest real stage at
# ~146s, so 900s is minutes of headroom above anything a working run does; the
# per-stage table exists for the stages that legitimately go long rather than to
# tune the ordinary ones.
# `${STAGE_TIMEOUT:-...}`, not `${VERIFY_STAGE_TIMEOUT:-...}`: this block is
# *below* the option loop, so a plain `STAGE_TIMEOUT=900` would overwrite what
# `--stage-timeout` just parsed. Measured before the fix: `--stage-timeout abc`
# was replaced by 900 and the run proceeded, which is the check-that-never-
# inspects-its-input shape this file exists to avoid.
STAGE_TIMEOUT="${STAGE_TIMEOUT:-900}"
# Per-stage overrides, keyed by stage name. Validated against STAGES below, so a
# typo here is a usage error rather than an entry that silently never applies —
# the `MALFORMED_ROWS`-style check this file applies to every other table.
#
# `flatpak-build` gets an hour because it is the one stage whose honest runtime
# is unbounded by this repository: a cold build downloads the runtime, the
# Wine base and the crate vendor tree, and 900s would fail a first run on a slow
# link. It is still a ceiling — the point is that a *hang* there is caught too.
declare -A STAGE_TIMEOUTS=(
    [flatpak-build]=3600
)
# Seconds between the TERM and the KILL. The TERM is what a well-behaved
# process needs to unwind (the GUI in `smoke-test` handles it and exits); the
# KILL is the backstop for one that ignores it.
STAGE_TIMEOUT_GRACE="${VERIFY_STAGE_TIMEOUT_GRACE:-10}"

# Both the option and the table are checked here, before any stage runs. A
# non-numeric ceiling is not a bad deadline, it is arithmetic on garbage: the
# `[ "$ceiling" -gt 0 ]` in `run_stage` would print "integer expression expected"
# and take the else branch, so the stage would run with the deadline *off* while
# the run reported nothing about it — the failure mode being "no deadline, and
# no word said", which is the state #103 is about. A key that names no stage is
# the same shape of silence the comment above the table warns about: the entry
# never applies, and the stage it was written for runs unbounded while the file
# reads as if it were covered. Both are usage errors (2), reported before any
# stage runs.
check_timeout_config() {
    local problem="" key
    # `run_with_deadline` waits on two specific pids with `wait -n`, which only
    # takes pid operands from bash 5.1. Older bash answers the path it does
    # understand — nothing — with 127, and 127 would be reported as the *stage*
    # failing. Say so here instead of letting every stage go red on a bash this
    # script cannot actually use.
    if [ "${BASH_VERSINFO[0]}" -lt 5 ] ||
       { [ "${BASH_VERSINFO[0]}" -eq 5 ] && [ "${BASH_VERSINFO[1]}" -lt 1 ]; }; then
        problem+="  bash ${BASH_VERSION} cannot express the stage deadline: \`wait -n\` needs 5.1\n"
    fi
    case "$STAGE_TIMEOUT" in
        ''|*[!0-9]*)
            problem+="  --stage-timeout ${STAGE_TIMEOUT} is not a whole number of seconds\n" ;;
    esac
    case "$STAGE_TIMEOUT_GRACE" in
        ''|*[!0-9]*)
            problem+="  VERIFY_STAGE_TIMEOUT_GRACE ${STAGE_TIMEOUT_GRACE} is not a whole number of seconds\n" ;;
    esac
    for key in "${!STAGE_TIMEOUTS[@]}"; do
        if ! printf '%s\n' "${STAGES[@]%%|*}" | grep -qx -- "$key"; then
            problem+="  STAGE_TIMEOUTS names ${key}, which is not a stage in STAGES\n"
        fi
        case "${STAGE_TIMEOUTS[$key]}" in
            ''|*[!0-9]*)
                problem+="  STAGE_TIMEOUTS[${key}]=${STAGE_TIMEOUTS[$key]} is not a whole number of seconds\n" ;;
        esac
    done
    if [ -n "$problem" ]; then
        printf 'verify.sh: the stage deadline is misconfigured\n' >&2
        printf '%b' "$problem" >&2
        printf '  a ceiling is whole seconds, and 0 disables it; the STAGE_TIMEOUTS\n' >&2
        printf '  keys must be stage names from STAGES, or the entry never applies.\n' >&2
        exit 2
    fi
}

check_timeout_config

# The process group of the stage currently running in `run_with_deadline`. Empty
# at every other moment, which is also when no INT/TERM trap is installed, so
# `kill_stage_group` is reachable only from the two handlers below.
STAGE_PGID=""

# A stage runs in its own process group (see `run_with_deadline`), which is what
# lets the deadline kill the *grandchildren* that actually hang. The same
# separation means the terminal's SIGINT — which goes to the foreground group —
# no longer reaches the stage, so `run_with_deadline` installs a handler that
# forwards it. That handler is installed *only* for the duration of a stage,
# deliberately: bash runs a trap after the foreground command returns, so a
# script-wide trap would turn every Ctrl-C during a long foreground wait into a
# deferred one. That is not theoretical — measured on this file, a SIGTERM sent
# while the script sat in the `flock` build-lock wait did not stop it, because
# the trap could not run until `flock` returned. Outside a stage the default
# disposition is the right one, and it is what the script had before #103.
kill_stage_group() {
    [ -n "$STAGE_PGID" ] || return 0
    kill -TERM -- "-$STAGE_PGID" 2>/dev/null
    kill -KILL -- "-$STAGE_PGID" 2>/dev/null
    STAGE_PGID=""
}

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
    # Same discipline for the timeout marker: `run_stage` sets it from the exit
    # status, and clearing it here is what stops a stage that timed out from
    # making the *next* stage's failure read as "timed out" too.
    STAGE_TIMED_OUT=""
    # Truncated here, so the log holds *this* stage's output and nothing else.
    # Without this, a stage that is skipped still finds the log a previous run
    # left behind, and `finish_skip`'s `echo_subchecks` prints it: a run with
    # `--skip-flatpak` displayed four `ok ...` lines under `SKIP smoke-test`,
    # byte-identical to a `smoke-test.log` written two hours earlier, from a
    # stage that this run did not run. That is the same defect as the rest of
    # this file — a result reported that was never produced — on the one path
    # that is supposed to report *no result*. Truncating rather than teaching
    # `finish_skip` not to read makes it unrepresentable at every call site.
    #
    # Checked, because that claim holds only if the write actually happened, and
    # this script is `set -uo pipefail` with no `set -e` (#53): a failed
    # truncation does not stop the run. Bash prints `Permission denied` on
    # stderr — it is not literally silent — but nothing acts on it, the stage
    # proceeds, and the run still exits 0, which restores exactly the state
    # described above. Measured: `SKIP smoke-test` printed a read-only
    # `smoke-test.log` from an earlier run and the run exited 0. The failure is
    # not "no log" — it is "another run's log, printed as this one's".
    #
    # Stopping is right rather than continuing without a log: the write fails
    # because the directory or the file is not writable, which would lose every
    # later stage's log too, and a run that discards its own evidence and exits
    # 0 is worse than one that does not start.
    STAGE_LOG="$LOGDIR/$1.log"
    if ! : > "$STAGE_LOG"; then
        printf 'verify.sh: begin %s — cannot empty %s\n' \
            "$STAGE" "${STAGE_LOG#"$ROOT"/}" >&2
        printf '  not writable, or its directory is not. It still holds whatever\n' >&2
        printf '  the last run put there, and a skipped stage prints that log\n' >&2
        printf '  verbatim (#52), so continuing would show the previous run as if\n' >&2
        printf '  it were this one.\n' >&2
        exit 2
    fi
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
    # Named separately from the generic FAIL, and printed *before* the log tail,
    # because the two failure modes need opposite responses: a stage that failed
    # has a defect in the code under test, a stage that timed out may have none
    # at all — it hung, or the ceiling is too tight for this machine. Without
    # this line the log tail is a truncated transcript of a process that was cut
    # off mid-sentence, and "it printed the last thing it was doing" reads as
    # "it failed there".
    if [ -n "$STAGE_TIMED_OUT" ]; then
        printf '     timed out: %s ran %ds with no result and was killed (ceiling %ss)\n' \
            "$STAGE" "$((SECONDS - STAGE_START))" "$STAGE_TIMED_OUT"
        # The elapsed time above can exceed the ceiling by up to the grace, since
        # the TERM-to-KILL wait is counted in whole seconds; saying so here stops
        # the reader concluding from "ran 3s (ceiling 2s)" that the deadline did
        # not take effect, which is the opposite of what happened.
        printf '     (the ceiling is when the killing starts, not when it finishes: the\n'
        printf '     TERM-to-KILL grace is up to %ss, so the elapsed time can exceed it)\n' \
            "$STAGE_TIMEOUT_GRACE"
        printf '     this is not a verdict about the code under test: the stage did not\n'
        printf '     finish. Re-run it alone, or raise the ceiling with --stage-timeout.\n'
    fi
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
    # The `|| return 1` here is load-bearing and stays, and that is measured
    # rather than defensive: redirections are performed before the command runs,
    # so a lock file the shell cannot open means `exec` is never called and the
    # fd variable is never assigned. Without the guard the function runs on and
    # the caller sees 0 — measured, `f() { local FD=""; exec {FD}>>/nonexistent/x;
    # return 0; }; f` exits 0 with `FD` empty — which is `acquire_flatpak_lock`
    # reporting a lock it does not hold, the one outcome it exists to prevent.
    # (It held before this pass; it is noted because the failure is invisible on
    # exactly the runs this harness cares about — `set -uo pipefail`, no `-e`,
    # #53.) With the guard, the same probe reports rc=1.
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
# Stage: doc — rustdoc resolves every intra-doc link (task #75)
#
# The lint is denied BY NAME, not by turning on `-D warnings`, and that is the
# decision rather than an oversight. The same run emits 14 `links to private
# item` warnings, and a doc comment that names a private helper is sometimes
# exactly the right thing to write — this project's doc comments cite test
# functions and private constants deliberately, as the evidence for a claim
# about them. Denying every rustdoc warning would fail those and teach the next
# author to delete the citation, which loses the evidence to save the lint.
#
# What it gates is the failure that is silent in the other direction: a broken
# intra-doc link degrades to an unlinked code span, so the rendered page reads
# correctly and only the author's intent is lost. `cargo doc` ran in no stage at
# all, so links that stopped resolving when their targets moved were invisible
# (#75). This stage runs the tool rather than grepping its output, because when
# it was written two of the ten errors it reported said `X is both a module and
# a macro` and not `unresolved link` — a grep for one diagnostic string is blind
# to the next class, which is the finding this stage exists to close. The ten
# were fixed before it landed (#75 and T-35); the count is not restated here,
# because a number in this comment is stale the moment the next author edits a
# doc comment, and this project has already filed that shape twice.
# ---------------------------------------------------------------------------
stage_doc() {
    RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" \
        cargo doc --workspace --no-deps
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
# A green `cargo test` is not evidence that the source was tested (#50).
#
# Cargo reuses a target artifact when the source's mtime is not newer than the
# artifact's, and `rsync -a` preserves mtimes, so this is reachable by ordinary
# work. It fails in the one direction nothing else here guards: a test added to
# a source file and then backdated is not in the binary cargo runs, and cargo
# prints a clean `358 passed; 0 failed` over a tree it never compiled. Measured
# with a test that *panics* — 358 passed, and the word `Compiling` nowhere in
# the output. The count is fine, the binary is not the source, and the summary
# says nothing about it.
#
# Section 3's rule ("a failure that contradicts the code is more likely a stale
# artifact than a bug") covers the false-*failure* direction and works because a
# contradiction makes someone look. A green count makes nobody look, which is
# why the fix has to be here rather than there.
#
# Two halves, and both are load-bearing. `cargo clean -p` removes the artifacts
# of the crates under test, so cargo must rebuild them from source and what runs
# is what is here. The `Compiling` check then fails the stage if that did not
# happen, so the forcing cannot stop working unnoticed — a different target
# dir, a cargo change, a flag that no longer does what it says. Forcing alone is
# a fix with nothing checking it; asserting alone would fail every legitimate
# incremental run in which there was nothing to rebuild.
#
# `clean -p` also removes the `[[bin]]` that `stage_cli` runs. Measured in a
# throwaway package rather than assumed: `cargo test` rebuilds the bin, so the
# later stages still find theirs.
#
# Not a general "is the cache fresh" check: it names the one mechanism that was
# measured to produce a false pass (D-41).
stage_test() {
    local rc=0 output status

    # One ok/FAIL line per crate, so a run that reused an artifact says so
    # rather than reporting green (the output contract's one-line-per-check
    # shape, echoed by `echo_subchecks`).
    require_compiled() {
        local crate="$1" out="$2"
        # A herestring, not `printf | grep -q`: with `set -o pipefail`, grep's
        # early exit on a match SIGPIPEs a printf that is still writing, and
        # the pipeline reports printf's 141 instead of grep's 0 — a FAIL on
        # output that contains the line. Measured: 59 failures in 60 runs of
        # the piped form against a 1.4 MB test log, 0 in 60 of this form.
        if grep -qE "^ +Compiling ${crate} v" <<<"$out"; then
            echo "ok   cargo compiled ${crate} from source before running its tests"
        else
            echo "FAIL cargo never compiled ${crate} — the binary it ran is not this source (#50)"
            rc=1
        fi
    }

    # `cargo test` exits 0 when it runs **nothing**, so the exit code alone is
    # not evidence that anything was tested: a crate whose only test is deleted
    # or `#[ignore]`d reports `running 0 tests / test result: ok. 0 passed` and
    # returns 0. Measured for #50 both ways — a crate with no `#[test]` at all,
    # and the same crate with its single test ignored. `require_compiled` does
    # not cover it either, because `Compiling` prints in both cases.
    #
    # So the run has a floor. It is deliberately *below* the current count
    # rather than equal to it: a test deleted on purpose (a tautology removed,
    # a case folded into another) is a legitimate edit that must not require
    # editing this file, while the wholesale disappearance the floor exists for
    # trips it by a mile. Raise it as the suite grows; the point is that a
    # collapse to zero cannot pass, not that this file tracks the exact number.
    # The floor is only meaningful together with `require_compiled`, which is
    # why it lives here beside it.
    require_tests_ran() {
        local label="$1" out="$2" floor="$3"
        local passed
        # Sum every `test result: ok. N passed` line across the run's binaries.
        passed="$(grep -oE '^test result: ok\. [0-9]+ passed' <<<"$out" \
            | grep -oE '[0-9]+' | paste -sd+ - | bc)"
        if [ -z "$passed" ]; then
            echo "FAIL $label ran no tests at all — cargo exited 0 with no 'test result' line (#50)"
            rc=1
        elif [ "$passed" -lt "$floor" ]; then
            echo "FAIL $label ran only $passed tests, floor is $floor — a collapse this large is not a green run (#50)"
            rc=1
        else
            echo "ok   $label ran $passed tests (floor $floor)"
        fi
    }

    # Configuration 1: the whole workspace, default target dir.
    if ! output="$(env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
            cargo clean -p gamehandler-core -p gamehandler 2>&1)"; then
        printf '%s\n' "$output"
        echo "FAIL cargo clean failed, so nothing below is known to test this source"
        return 1
    fi
    output="$(env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET cargo test 2>&1)"
    status=$?
    printf '%s\n' "$output"
    [ "$status" -eq 0 ] || return 1
    require_compiled gamehandler-core "$output"
    require_compiled gamehandler "$output"
    require_tests_ran "the workspace suite" "$output" 900

    # Configuration 2: the core crate alone, in its own target dir (task #27),
    # which needs its own clean for the same reason.
    if ! output="$(env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
            CARGO_TARGET_DIR="$ROOT/target/verify-narrow" \
            cargo clean -p gamehandler-core 2>&1)"; then
        printf '%s\n' "$output"
        echo "FAIL cargo clean failed in the narrow target dir"
        return 1
    fi
    output="$(env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
        CARGO_TARGET_DIR="$ROOT/target/verify-narrow" \
        cargo test -p gamehandler-core 2>&1)"
    status=$?
    printf '%s\n' "$output"
    [ "$status" -eq 0 ] || return 1
    require_compiled gamehandler-core "$output"
    require_tests_ran "the core crate alone" "$output" 500

    return "$rc"
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
    #
    # **The unknown arm, and on its own it gates nothing.** Through T-08 this was
    # the stage's *only* `--launch` assertion, and the id it launches is not in
    # the library: that is the arm the pre-T-07 stub already got right, so
    # deleting `launch_failure`'s unknown-id path would have left this check
    # green. The stage's own description ("the headless CLI
    # --list/--launch/--version, against a library it must read") was true and
    # vacuous for `--launch`, which never read a game. `cli-launch-known` below
    # is the missing half; this one stays because the refusal is still contract.
    rc=0; cli_run "$two" --launch this-id-is-not-in-the-library >"$out" 2>"$err" || rc=$?
    if [ "$rc" -ne 0 ] && grep -q "no game with id" "$err"; then
        echo "ok   cli-launch-unknown (exit $rc, reason on stderr)"
    else
        echo "FAIL cli-launch-unknown: an unknown id must exit non-zero with the reason on"
        echo "     stderr; got exit $rc, stderr: $(head -n1 "$err")"
        failures=$((failures + 1))
    fi

    # --- cli-launch-known ---
    #
    # The check #31 left to a unit test. A **known** id, in a library the binary
    # really reads, whose game is a native title pointing at `/bin/true` — the
    # same fixture Architecture's `a_real_launch_records_the_game_as_played`
    # (`main.rs:3350`) drives in process, which is why the shape is known to work
    # rather than hoped to. Three properties, and the third is the one that is
    # not free:
    #
    #   1. the id resolves (a lookup, not a refusal);
    #   2. it exits **0** — `/bin/true` terminates successfully inside the grace
    #      period, and `LaunchedGame::failure` returns `None` for exit 0 exactly
    #      as Python's does, so this is the `Ok(None)` arm and the only arm
    #      `launch_report` gives a zero;
    #   3. **`last_played` moved, read back from the file.** This is the
    #      assertion the rc cannot make. `mark_played` is the line between the
    #      launch and the watch, and **deleting the whole call left the suite
    #      green at 298 passed** — Architecture found that mutation surviving,
    #      and the seam `launch_game_at` exists because of it. rc 0 alone passes
    #      on a binary that launches the game and records nothing, so a check
    #      written for #31 with only the exit code would have re-opened T-07.
    #
    # The value is read from `games.json` and not from anything the binary
    # printed, and it is compared before/after on the same file. A state where
    # those two assertions disagree is reachable with the real binary and no
    # rebuild at all: with the config directory unwritable, the launch succeeds
    # and exits 0 while the save fails, stderr carries
    # `could not record “…” as played: Permission denied`, and this check must
    # go red on it. That was the DoD, and it is the reason (3) is here.
    local known="$tmp/known" library played_before played_after
    mkdir -p "$known/gamehandler" || { rm -rf "$tmp"; return 1; }
    cat >"$known/gamehandler/games.json" <<'JSON'
[{"id": "33333333333333333333333333333333", "name": "Native Probe", "kind": "linux", "exe_path": "/bin/true", "last_played": 0.0}]
JSON
    library="$known/gamehandler/games.json"

    # last_played_value <file> — the number, or `absent` when the key is gone.
    # One definition for both reads, so the before and the after cannot be
    # extracted by two spellings that disagree.
    last_played_value() {
        local value
        value="$(grep -oE '"last_played": *-?[0-9][0-9.eE+-]*' "$1" 2>/dev/null \
            | head -n1 | sed 's/.*: *//')"
        printf '%s\n' "${value:-absent}"
    }

    if [ ! -x /bin/true ]; then
        echo "FAIL cli-launch-known: /bin/true is not executable, so a native title"
        echo "     cannot be launched without a runner. This check needs it, and"
        echo "     says so rather than skipping: a check that skips is a check that"
        echo "     reports the library as reachable without reading it."
        failures=$((failures + 1))
    else
        played_before="$(last_played_value "$library")"
        rc=0; cli_run "$known" --launch 33333333333333333333333333333333 >"$out" 2>"$err" || rc=$?
        played_after="$(last_played_value "$library")"
        if [ "$rc" -eq 0 ] && [ "$played_after" != "$played_before" ] \
            && [ "$played_after" != "absent" ] && [ "$played_after" != "0.0" ]; then
            echo "ok   cli-launch-known (exit 0, last_played $played_before -> $played_after)"
        else
            echo "FAIL cli-launch-known: a known native title (/bin/true) in a library"
            echo "     at \$XDG_CONFIG_HOME/gamehandler/games.json must exit 0 AND move"
            echo "     last_played in that file; got exit $rc, last_played"
            echo "     $played_before -> $played_after and:"
            echo "     --- stdout ---"; sed 's/^/     | /' "$out"
            if [ -s "$err" ]; then echo "     --- stderr ---"; sed 's/^/     | /' "$err"; fi
            if [ "$rc" -eq 0 ] && [ "$played_after" = "$played_before" ]; then
                echo "     the exit code is right and the record did not move: the game"
                echo "     launched and nothing wrote it down, so a shortcut reports"
                echo "     success and the library never learns the title was played."
            fi
            failures=$((failures + 1))
        fi
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
#
# PYTHONDONTWRITEBYTECODE: without it the stage can grade a *stale* revision.
# CPython validates a cached .pyc by the source's mtime, truncated to whole
# seconds, plus its size — so an edit that keeps the file's size and lands in
# the same second as the previous compile is not detected, and the cached
# bytecode for the previous revision is executed. Measured while writing the
# #93 check: renaming a dict key `"T-12"` to `"T-99"` (same byte count) and
# restoring it left `python3 -m unittest` loading the mutated module while the
# file on disk held the original — the suite reported the previous revision's
# verdict, both ways round. That is #92's shape (a measurement that cannot
# fail) sitting in the runner rather than in a check, and it bites hardest on
# exactly the same-size edits this project makes constantly. `-B` alone only
# stops writes; with no bytecode written there is nothing stale to read, but
# any `__pycache__` left behind by earlier runs must be removed once by hand.
# ---------------------------------------------------------------------------
stage_python() {
    env -u DISPLAY -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
        PYTHONDONTWRITEBYTECODE=1 \
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
    #
    # The count is a *floor*, not a report (BUG-16b). This half decides on
    # `missing` alone, and `missing` can only be non-zero for a URL the loop
    # actually read: make the lock registry-only, or change the shape
    # `source = "git+…"` is written in, and the `sed` yields nothing, the loop
    # body never runs, `count=0 missing=0`, and the stage prints
    # `ok cargo-sources` having compared the committed file against a
    # regenerated one and covered **no source at all**. The count was printed
    # and never required, which is the same shape as BUG-10: an exit status
    # standing in for a check on what ran. So it is required, and zero is a
    # failure rather than a pass — the `sed` above is the load-bearing read here,
    # and a `sed` that stopped matching must not be indistinguishable from a lock
    # file with no git sources in it. (A lock with none is not a state this
    # project can be in: libcosmic is a git dependency, which is why
    # build-aux/flatpak/wrap-cargo-sources.py exists at all.)
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
    if [ "$count" -eq 0 ]; then
        echo "FAIL Cargo.lock yielded no 'source = \"git+…\"' lines, so this half covered"
        echo "     nothing. That is either a lock file with no git dependencies (not a"
        echo "     state this project can be in — libcosmic is one) or the read above"
        echo "     having stopped matching, and the two are indistinguishable here."
        echo "     Fix the read: the pattern is anchored to the start of the line."
        return 1
    fi
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

    # Which copy gets validated is part of the *verdict*, not a line above it
    # (BUG-16a). The fallback below used to run silently and the stage still
    # returned 0, so a run in which `flatpak-build` failed under `--keep-going`
    # — or in which build-flatpak/ is a leftover from an earlier revision —
    # printed `ok desktop-metainfo` after validating the repository's *sources*.
    # A reader takes "the installed copies validate" from that line, and nothing
    # in the run produced it: the label survived the change of subject, which is
    # the whole of the defect.
    #
    # So the subject is tracked. A fallback still validates the sources — that
    # check is real and the sub-check lines say which files they read — but the
    # stage cannot then report ok, because the thing it exists to validate (the
    # desktop entry and the metainfo *as installed*) was not looked at. It
    # returns the same pair of skip codes `flatpak-contents` uses for the same
    # situation (98 when the caller asked for the missing tree with
    # --skip-flatpak, 99 when a prerequisite is missing), so the run's exit
    # status distinguishes "you asked not to build" from "the build is not
    # there".
    local installed=1
    [ -f "$desktop" ] || { desktop="$ROOT/data/$APP_ID.desktop"; installed=0; }
    [ -f "$metainfo" ] || { metainfo="$ROOT/data/$APP_ID.metainfo.xml"; installed=0; }
    echo "desktop:  ${desktop#"$ROOT"/}"
    echo "metainfo: ${metainfo#"$ROOT"/}"
    if [ "$installed" -eq 0 ]; then
        echo "NOT the installed copies: no"
        echo "  ${share#"$ROOT"/}/applications/$APP_ID.desktop or"
        echo "  ${share#"$ROOT"/}/metainfo/$APP_ID.metainfo.xml."
        echo "The repository's sources are validated below; the installed copies"
        echo "were NOT read, so this stage is a SKIP and not a pass."
    fi

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
    if [ "$installed" -eq 0 ]; then
        [ "${SKIP_FLATPAK:-0}" -eq 1 ] && return 98
        return 99
    fi
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
    "data/icons/hicolor/64x64/apps/$APP_ID.png|share/icons/hicolor/64x64/apps/$APP_ID.png"
    "data/icons/hicolor/128x128/apps/$APP_ID.png|share/icons/hicolor/128x128/apps/$APP_ID.png"
    "data/icons/hicolor/256x256/apps/$APP_ID.png|share/icons/hicolor/256x256/apps/$APP_ID.png"
    "data/icons/hicolor/512x512/apps/$APP_ID.png|share/icons/hicolor/512x512/apps/$APP_ID.png"
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
    # THE DENOMINATOR, and why this stage cannot be trusted without one. The
    # source count below is a count of arms calling `pending_page`, and that
    # number is *supposed* to reach zero at T-19 — so "zero" has to mean
    # "nothing dispatches to the placeholder", not "the file was not read". It
    # cannot tell those apart on its own: a failed grep prints 0 through
    # `wc -l` exactly as a successful grep with no matches does, and the
    # pipeline's status is discarded. Measured: the same pipeline against a
    # nonexistent path gives `0` and rc=2, silently. So the zero-placeholder
    # tree — the one T-19 verifies — would read a missing `main.rs` as a fully
    # ported one and `ok` it.
    #
    # `in_dispatch` is the file's page-dispatch arm count, from the
    # `match self.state.page` anchor to EOF. It is non-zero whenever the
    # dispatch was actually found, so requiring it non-zero makes the zero
    # above a finding rather than an absence. It counts from the anchor rather
    # than matching `Page::X =>` file-wide because the file has a *second*
    # `Page::X =>` match (the icon names at ~:658, before the anchor); file-wide
    # the number is 13, and a denominator that a second match can hold up is
    # not evidence that this match was read.
    local dispatch_src="$ROOT/crates/app/src/main.rs"
    if [ ! -r "$dispatch_src" ]; then
        echo "FAIL cannot read ${dispatch_src#"$ROOT"/}, so this stage cannot tell a"
        echo "     ported tree from an unread one — both give a count of 0."
        rc=1
        return "$rc"
    fi

    local in_source in_binary in_dispatch
    in_source="$(grep -oE '^[[:space:]]*Page::[A-Za-z]+ => pending_page\(Page::' \
        "$dispatch_src" | wc -l | tr -d '[:space:]')"
    in_binary="$(grep -ao -- 'This page has not been ported yet (' \
        "$tree/bin/gamehandler" | wc -l | tr -d '[:space:]')"
    # The anchor is matched from the start of the line AND requires the opening
    # brace. Both are load-bearing, and both were found by breaking it rather
    # than by reasoning: with the pattern as a bare substring, renaming the
    # dispatch (`match self.state.page_renamed`) still matched and the count
    # stayed at 6, so the denominator survived the very edit it exists to
    # detect. The line-start anchor also skips the doc comment at ~:902 that
    # quotes this anchor in prose — the same trap `pending_pages.rs` hit when an
    # unanchored search found its own documentation first and parsed nothing.
    in_dispatch="$(awk '/^[[:space:]]*match self\.state\.page[[:space:]]*\{/{found=1}
        found && /^[[:space:]]*Page::[A-Za-z]+ =>/{count++}
        END{print count+0}' "$dispatch_src")"

    if [ "${in_dispatch:-0}" -eq 0 ]; then
        echo "FAIL found no page-dispatch arms in ${dispatch_src#"$ROOT"/}"
        echo "     no 'match self.state.page' was found, or it has no arms under it."
        echo "     The placeholder counts below cannot mean anything without it: with"
        echo "     no dispatch read, 'no arm calls pending_page' and 'the file was not"
        echo "     read' are the same observation, and the second is not evidence."
        echo "     If the dispatch moved out of this file, point this stage at it."
        rc=1
    elif [ "$in_source" -gt 0 ] && [ "$in_binary" -gt 0 ]; then
        echo "ok   the shipped binary carries the placeholder text, as the source does"
        echo "     ($in_source arm(s) dispatch to pending_page; $in_binary occurrence(s) in the ELF)"
        echo "     both sides are non-zero, so this checked that the two AGREE that"
        echo "     placeholder pages exist — NOT which pages they are. The check that"
        echo "     names pages is crates/app/tests/pending_pages.rs, which is reported"
        echo "     by the test stage; this half is only the artifact cross-check."
    elif [ "$in_source" -eq 0 ] && [ "$in_binary" -eq 0 ]; then
        echo "ok   no page dispatch arm calls pending_page, and the shipped binary"
        echo "     carries no placeholder text — T-19's end state"
        echo "     $in_dispatch dispatch arm(s) were read, so this is the dispatch having"
        echo "     no placeholder in it, NOT a parse that found nothing. The earlier"
        echo "     wording here read 'every page is ported in the source', which is an"
        echo "     inference from absence and was stronger than the check: an arm that"
        echo "     draws nothing, or draws text(\"TODO\"), also fails to call"
        echo "     pending_page. The ELF half is an absence too. So this establishes"
        echo "     that the placeholder is gone and says nothing about whether the"
        echo "     pages render anything correct — the check that names pages is"
        echo "     crates/app/tests/pending_pages.rs, and parity is T-19's."
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

    # --- the installed binary actually runs (PKG-04) ------------------------
    #
    # Everything else in this stage READS the binary: the five-file loop below
    # `cmp`s files, and the ELF check above greps the executable for a string.
    # Nothing had ever executed it — `grep -n 'flatpak run\|flatpak build\|
    # flatpak install\|flatpak info\|flatpak kill' scripts/verify.sh` returned
    # nothing (PKG-04, re-checked 2026-09-13, rc 1) — so the chain could prove
    # the binary landed at /app/bin/gamehandler with the right bytes and still
    # ship one that does not start: a missing runtime, a bad Exec, an unresolved
    # .so, a wrong base. None of those is visible to `cmp`. The one stage that
    # did execute something — smoke-test, scripts/smoke-test.sh — could be
    # executing a *previously installed* build instead of this tree's (PKG-01,
    # fixed in the same pass, and the two are a pair: the execution, and the
    # artefact it belongs to).
    #
    # `flatpak build <dir>`, not `flatpak run`: this stage reads build-flatpak/,
    # which flatpak-builder produced and which nothing in this chain installs
    # (`flatpak install` appears nowhere in it). `flatpak build` assembles a
    # sandbox around the tree without installing it — the same way
    # scripts/smoke-test.sh reaches this binary in its build-dir mode — and
    # `/app/bin/gamehandler` under `<dir>` is `<dir>/files/bin/gamehandler`.
    # Measured, not assumed: that file is 29470184 bytes in this tree, and
    # `flatpak build build-flatpak /app/bin/gamehandler --version` printed
    # `GameHandler 0.8.0` and exited 0.
    #
    # `--version` and `--list`, with the display variables unset. The unsets are
    # the three smoke-test's `none` mode uses, for the reason it gives: a check
    # must not pass because the caller's display leaked into the sandbox. The
    # CLI is the right path to exercise that way because DECISIONS D-12
    # dispatches it before any window or event loop, so it needs no compositor.
    # Measured: `--list` printed `GameHandler: the library is empty` and exited
    # 0. What `--list` prints is NOT examined, exactly as smoke-test's
    # cli-list-exits-0 does not examine it; the assertion is that the verb is
    # reachable and the process exits, not what it says.
    #
    # The version is matched as a SHAPE and not against `0.8.0`. The number
    # lives in Cargo.toml, and a second copy here would be a pin that goes stale
    # on the next bump — it would then fail a correct build, which is how a gate
    # teaches its reader to ignore it. The shape is enough for what this half
    # asserts: that the binary started and answered as GameHandler.
    #
    # What a FAIL here does NOT distinguish, said plainly because the output
    # looks the same either way: a `flatpak build` that cannot create its
    # sandbox at all (bwrap denied user namespaces, the runtime not installed
    # locally) is reported as the binary failing to run. The captured output is
    # printed with the failure and names which of the two happened; the exit
    # status does not. The common case is out of reach here — a runtime missing
    # locally fails smoke-test first, which runs earlier and needs the same
    # sandbox.
    local exec_missing=0 exec_rc=0 exec_out="" list_rc=0 list_out=""
    if ! require_tool flatpak "to run the built gamehandler binary"; then
        exec_missing=1
    else
        exec_out="$(flatpak build --unset-env=DISPLAY --unset-env=WAYLAND_DISPLAY \
            --unset-env=WAYLAND_SOCKET "$BUILD_DIR" /app/bin/gamehandler \
            --version 2>&1)" || exec_rc=$?
        list_out="$(flatpak build --unset-env=DISPLAY --unset-env=WAYLAND_DISPLAY \
            --unset-env=WAYLAND_SOCKET "$BUILD_DIR" /app/bin/gamehandler \
            --list 2>&1)" || list_rc=$?

        if [ "$exec_rc" -ne 0 ]; then
            echo "FAIL the installed binary does not run: /app/bin/gamehandler"
            echo "     --version exited $exec_rc"
            printf '%s\n' "$exec_out" | tail -n 10 | sed 's/^/     | /'
            rc=1
        elif ! grep -qE '^GameHandler [0-9]+\.[0-9]+\.[0-9]+$' <<<"$exec_out"; then
            echo "FAIL the installed binary ran but did not answer as GameHandler:"
            echo "     /app/bin/gamehandler --version printed"
            printf '%s\n' "$exec_out" | tail -n 10 | sed 's/^/     | /'
            rc=1
        elif [ "$list_rc" -ne 0 ]; then
            echo "FAIL the installed binary runs, but /app/bin/gamehandler"
            echo "     --list exited $list_rc"
            printf '%s\n' "$list_out" | tail -n 10 | sed 's/^/     | /'
            rc=1
        else
            echo "ok   the installed binary runs: /app/bin/gamehandler --version ->"
            echo "     $(head -n1 <<<"$exec_out") (exit 0; --list also exited 0)"
            echo "     sha256 $(sha256sum "$tree/bin/gamehandler" | cut -d' ' -f1)"
            echo "     which is the artefact smoke-test executes in build-dir mode"
            echo "     (PKG-01), so both stages' results belong to the same file."
            echo "     A start check, not a behavioural one: it proves the binary"
            echo "     loads, resolves its libraries and reaches its CLI. What the"
            echo "     verbs do is the cli stage's, against a library it owns."
        fi
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

    # A failure above is a failure whatever else could not run, so it is tested
    # first — the same order the manifest half uses above. Then the execution
    # half's own prerequisite. 99, not 98: flatpak-builder cannot run without
    # flatpak, so an absent flatpak is never a consequence of --skip-flatpak,
    # and a run that ends here did not verify that the binary runs.
    [ "$rc" -eq 0 ] || return 1
    [ "$exec_missing" -eq 0 ] || return 99
    return 0
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

# Run a stage function under its deadline and return its exit status, or 124 if
# the deadline expired and the stage had to be killed (#103).
#
# `timeout` is the obvious tool here and cannot be used: it execs a program, and
# a stage is a shell function. Measured — `timeout 5 stage_build` fails with
# "failed to run command 'stage_build': No such file or directory", rc 127, which
# `run_stage` would have reported as the *stage* failing. That was this code's
# first version, and the probe caught it.
#
# But `timeout` has a second property this genuinely needs, and it is not
# cosmetic: it puts the child in its own process group and signals the whole
# group. Killing only the stage's own shell leaves the processes that actually
# hang. The case that filed #103 was two orphaned
# `netpaths::tests::unquote_matches_cpython` binaries spinning at 99.1% CPU for
# six hours — those are *grandchildren* of the stage, children of `cargo test`,
# and only a group signal reaches them. So the stage is forked under job control
# (`set -m`), which makes the job its own process-group leader, and the group is
# what gets signalled.
#
# Two consequences, both deliberate:
#   * the stage is no longer in the script's process group, so a terminal Ctrl-C
#     does not reach it — the INT/TERM traps above forward it, which is what
#     keeps Ctrl-C meaning what it meant before;
#   * the stage runs in a subshell, so a global it assigns does not survive.
#     No stage assigns one: they report through their exit status and their log.
#     `trap - EXIT` clears the inherited flatpak-lock release, which a subshell
#     would otherwise run on its own exit and hand the lock back mid-stage.
run_with_deadline() {
    local fn="$1" ceiling="$2"
    local rc=0

    # Forward Ctrl-C and SIGTERM into the stage's group for exactly as long as
    # the stage is running — see `kill_stage_group` for why these are installed
    # here rather than at the top of the file. 130/143 are the conventional
    # 128+signal codes; the EXIT trap that releases the flatpak lock still runs.
    trap 'kill_stage_group; exit 130' INT
    trap 'kill_stage_group; exit 143' TERM

    set -m
    ( trap - EXIT; "$fn" ) >"$STAGE_LOG" 2>&1 &
    STAGE_PGID=$!
    set +m

    # `exec`, so this subshell *is* the sleep: one process, no `sleep` child left
    # orphaned when the stage finishes first and the watchdog is killed.
    ( exec sleep "$ceiling" ) &
    local napper=$!

    # Whichever finishes first. If it is the sleep, the stage has overrun.
    wait -n "$STAGE_PGID" "$napper"; rc=$?

    if kill -0 "$STAGE_PGID" 2>/dev/null; then
        kill -TERM -- "-$STAGE_PGID" 2>/dev/null
        local waited=0
        while kill -0 "$STAGE_PGID" 2>/dev/null && [ "$waited" -lt "$STAGE_TIMEOUT_GRACE" ]; do
            sleep 1
            waited=$((waited + 1))
        done
        # The backstop, for the stage that ignores TERM — which is not
        # hypothetical: a `trap '' TERM` child is the obvious way a Wine or
        # flatpak-builder helper ends up unkillable, and it is what the probe
        # for this used.
        kill -KILL -- "-$STAGE_PGID" 2>/dev/null
        wait "$STAGE_PGID" 2>/dev/null
        STAGE_TIMED_OUT="$ceiling"
        rc=124
    fi

    kill "$napper" 2>/dev/null
    wait "$napper" 2>/dev/null
    STAGE_PGID=""
    # Back to the default disposition for everything between stages. Without
    # this a Ctrl-C during the build-lock wait, a `git status` in a stage's
    # epilogue, or the summary would be deferred until that command returned.
    trap - INT TERM
    return "$rc"
}

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
    # The ceiling for *this* stage: the per-stage override if the table has one,
    # the option default otherwise (#103).
    local ceiling="${STAGE_TIMEOUTS[$name]:-$STAGE_TIMEOUT}"
    if [ "$ceiling" -gt 0 ]; then
        run_with_deadline "$fn" "$ceiling" || rc=$?
    else
        # 0 disables the deadline, and what disabling it has to buy is the
        # *unmodified* call — same process group, same Ctrl-C behaviour, no
        # wrapper — so this is the only path that still runs the stage in the
        # foreground. It is the escape hatch for debugging a hanging stage by
        # hand, which is exactly when a deadline is in the way.
        "$fn" >"$STAGE_LOG" 2>&1 || rc=$?
    fi
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
run_stage doc
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
    printf '\nNOTE: the working tree changed during this run.\n'
    printf 'The likely cause is another agent editing this shared checkout while the run\n'
    printf 'was in flight — four agents work in this one tree (D-49), so this fires on\n'
    printf 'ordinary work rather than on a fault. The diff below names the paths, and\n'
    printf 'the paths are what identify who changed them.\n'
    printf '\n'
    printf 'verify.sh is separately required to be read-only with respect to tracked\n'
    printf 'files (T-17). Read that as the check, not as the verdict: a path below that\n'
    printf 'verify.sh writes is a violation of it; a path it never touches belongs to\n'
    printf 'whoever else was editing, and this note is then telling you the run raced\n'
    printf 'them rather than that the gate wrote outside itself.\n'
    printf '\n'
    printf 'Either way the result is provisional. A run over a tree that moved is not\n'
    printf 'attributable to any single tree state (D-45), so re-run on a still tree\n'
    printf 'before quoting it:\n'
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
