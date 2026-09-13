#!/usr/bin/env bash
#
# GameHandler — the automatic caller for scripts/verify.sh (PKG-02).
#
# Why this file exists, in the audit's words: "Nothing in the repository runs the
# verification chain automatically." scripts/verify.sh and scripts/parity-walk.sh
# were hand-invoked only — `grep -rn 'verify.sh'` over the tracked tree found no
# caller outside scripts/ and docs/ — so a packaging regression is caught only
# when a human remembers to run a 2,000-line script, and there is no artefact —
# no CI log, no hook output — that shows the chain ran against a given commit
# (docs/audit/PACKAGING.md, PKG-02).
#
# This is that caller. It is deliberately thin, and the thinness is the design:
# it picks the flag set, names the commit it ran against, tees the whole run to
# a file under target/, and exits with verify.sh's own status. It decides
# nothing about stages and holds no list of them. A second copy of the stage
# list is exactly the drift verify.sh's STAGES array exists to stop — the two
# copies that used to exist cost tasks #29 and #48, and both read green.
#
#   scripts/ci.sh                 the gate: scripts/verify.sh --skip-flatpak
#   scripts/ci.sh --full          the whole chain, flatpak-build and smoke-test
#                                 included (minutes; needs flatpak-builder and
#                                 the org.freedesktop.Sdk runtime)
#   scripts/ci.sh --keep-going    pass --keep-going through: run every stage and
#                                 report the full table instead of stopping at
#                                 the first failure
#
# What the default covers, measured rather than inferred — `scripts/ci.sh` on
# 2026-09-13 at 9f07c41e printed ten `ok` lines: build, clippy, doc, test (both
# feature configurations), cli, oracle-freshness, python-tests, cargo-sources,
# desktop-metainfo and flatpak-contents. `--skip-flatpak` asks verify.sh to skip
# flatpak-build and smoke-test, which it reports as skips the caller asked for —
# a legitimate exit 0 by D-31.
#
# What it does NOT cover, and this is the part worth reading twice: the OTHER
# two stages of the locked flatpak group are skipped only while no build tree
# exists. On a checkout that already has build-flatpak/ — every developer's, and
# the state this tree is in — desktop-metainfo and the installed-copies half of
# flatpak-contents ran in that same 2026-09-13 run, against THAT tree, which may
# predate the commit in the log. verify.sh's flatpak-contents stage says out loud
# that it cannot date the tree (its marker is the built binary, not an mtime), so
# a green gate run is not a statement about the Flatpak this commit produces.
# `--full` is. The gate is the cheap check, not the whole one.
#
# A skip for a MISSING PREREQUISITE is a different thing again, and it is why
# exit 3 is called out by name below: it is the code a reader mistakes for green.
#
# Why there is no .github/workflows file here, which is the other half of
# PKG-02's fix text. The chain needs flatpak-builder, the org.freedesktop.Sdk
# 25.08 runtime and the SDK's rust-stable extension (DECISIONS D-10), and the
# host dependency set for the cargo build is written down nowhere in this tree:
# the development machine and the SDK both provide a system Rust and the system
# libraries, and rust-toolchain.toml exists only for contributors who use rustup.
# A workflow authored from here would be a guess at a package list for a hosted
# runner, and a gate that reds on its first run for a reason that is not the code
# teaches its reader to ignore it — the argument verify.sh's stage-deadline note
# already makes about ceilings a real stage can hit. So the entry point is the
# deliverable, and wiring it is one line on any machine that has the toolchain:
#
#   CI step:            scripts/ci.sh            (--full on a runner with flatpak)
#   pre-push hook:      ln -s ../../scripts/ci.sh .git/hooks/pre-push
#
# The hook is deliberately NOT installed by this script: this is a shared
# checkout (four agents, D-49), and `git config core.hooksPath` — or a file in
# .git/hooks — is repository state that a verification script should not change
# behind the other three. The command above is the whole of the installation.
#
# The run is recorded, because "no artefact that shows the chain ran against a
# given commit" is the impact PKG-02 names. target/ is gitignored, and the file
# is named for the commit, so the question is answerable from the tree:
#
#   target/ci/<YYYYmmddTHHMMSSZ>-<HEAD sha, 12>.log
#
# Exit status: verify.sh's own (see the "Exit codes" note in its header).
#   0 = every stage passed, or was skipped because the caller asked
#   1 = a stage failed
#   2 = usage error (here or in verify.sh)
#   3 = the run was INCOMPLETE — a stage was skipped for a missing prerequisite,
#       so this run did not verify what its stage list claims
#
# Verified by breaking the thing it is supposed to catch, on 2026-09-13. A
# failing test appended to tests/test_settings.py gave `scripts/ci.sh` exit 1,
# a `FAIL python-tests` line, and the planted assertion in the log; the file
# restored, the same command gave exit 0 with the ten `ok` lines above. Both
# runs are in target/ci/ named for the commit they saw.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
    cat <<'EOF'
usage: scripts/ci.sh [options]

  --full          run the whole chain: drop --skip-flatpak, so flatpak-build and
                  smoke-test run too. Minutes, and needs flatpak-builder.
  --keep-going    pass --keep-going to verify.sh: report the full table of
                  stage results instead of stopping at the first failure.
  -h, --help      this text.

Default (no options) is the gate: scripts/verify.sh --skip-flatpak.

Every run is logged to target/ci/<timestamp>-<commit>.log and the exit status is
verify.sh's own: 0 pass, 1 a stage failed, 2 usage error, 3 the run was
incomplete (a stage skipped for a missing prerequisite — NOT a pass).
EOF
}

MODE="gate"
PASSTHRU=()
while [ $# -gt 0 ]; do
    case "$1" in
        --full)       MODE="full"; shift ;;
        --keep-going) PASSTHRU+=(--keep-going); shift ;;
        -h|--help)    usage; exit 0 ;;
        *)            echo "ci.sh: unknown option: $1" >&2; exit 2 ;;
    esac
done

VERIFY="$ROOT/scripts/verify.sh"
if [ ! -f "$VERIFY" ]; then
    echo "ci.sh: $VERIFY is missing, so there is nothing to run." >&2
    exit 2
fi

# The commit the result belongs to, and whether the tree it ran on was the
# committed one. A dirty tree is not a failure — this is a working checkout, and
# the run is still the run — but a result that does not say which tree it saw is
# the thing PKG-02 is about, so it is recorded either way.
COMMIT="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)"
BRANCH="$(git -C "$ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
TREE_STATE="clean"
[ -n "$(git -C "$ROOT" status --porcelain 2>/dev/null)" ] && TREE_STATE="dirty (uncommitted changes present)"

LOGDIR="$ROOT/target/ci"
if ! mkdir -p "$LOGDIR"; then
    echo "ci.sh: cannot create ${LOGDIR#"$ROOT"/}" >&2
    exit 2
fi
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
LOG="$LOGDIR/$STAMP-${COMMIT:0:12}.log"

ARGS=()
[ "$MODE" = "gate" ] && ARGS+=(--skip-flatpak)
# Under `set -u` an empty array expansion is fine on the bash verify.sh already
# requires (5.1+ for `wait -n`, checked in check_timeout_config), so this needs
# no guard for the no---keep-going case.
ARGS+=("${PASSTHRU[@]}")

printf 'ci.sh: %s\n' "$MODE"
printf '  commit: %s (%s)\n' "${COMMIT:-unknown}" "$TREE_STATE"
printf '  branch: %s\n' "${BRANCH:-unknown}"
printf '  command: scripts/verify.sh %s\n' "${ARGS[*]}"
printf '  log: %s\n\n' "${LOG#"$ROOT"/}"

{
    printf 'commit: %s (%s)\n' "${COMMIT:-unknown}" "$TREE_STATE"
    printf 'branch: %s\n' "${BRANCH:-unknown}"
    printf 'date:   %s\n' "$STAMP"
    printf 'mode:   %s\n' "$MODE"
    printf 'command: scripts/verify.sh %s\n\n' "${ARGS[*]}"
} > "$LOG"

# `tee -a`, and PIPESTATUS rather than `$?`, so the status is verify.sh's and not
# tee's. The whole run is in the log and on the terminal: a CI runner shows the
# first and a developer wants the second.
rc=0
"$VERIFY" "${ARGS[@]}" 2>&1 | tee -a "$LOG"
rc="${PIPESTATUS[0]}"

printf '\nci.sh: verify.sh exited %d — log: %s\n' "$rc" "${LOG#"$ROOT"/}"
case "$rc" in
    0) printf 'ci.sh: PASS (%s)\n' "$MODE" ;;
    1) printf 'ci.sh: FAIL — a stage failed. That is a defect to fix, and the\n'
       printf '  stage is named by the `FAIL <stage>` line above.\n' ;;
    3) printf 'ci.sh: INCOMPLETE — at least one stage was skipped for a missing\n'
       printf '  prerequisite, so this run did NOT verify what its stage list\n'
       printf '  claims. Every line above may still be green. This is not a pass:\n'
       printf '  install what the SKIP line names, or run --full on a machine that\n'
       printf '  has it.\n' ;;
    2) printf 'ci.sh: usage error — verify.sh rejected the option set above.\n' ;;
    *) printf 'ci.sh: exit %d, which verify.sh does not document.\n' "$rc" ;;
esac
exit "$rc"
