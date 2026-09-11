#!/usr/bin/env bash
#
# Headless smoke test for the built GameHandler Flatpak.
#
# Design: docs/migration/packaging.md §7.2. The app renders with
# `iced_tiny_skia` (DECISIONS D-11), so this needs a **display server and
# nothing else** — no Vulkan loader, no ICD, no lavapipe, no /dev/dri. A
# compositor socket is the entire external requirement.
#
# Checks, each reported as `ok|FAIL|SKIP <name>`:
#
#   cli-version             `--version` exits 0 with the expected shape, with
#                           no display (DECISIONS D-12: the CLI never
#                           initialises the GUI).
#   cli-list-exits-0        `--list` *exits 0* with no display (D-12). Named
#                           for exactly what it asserts: its output is not
#                           examined at all, and a `--list` that prints anything
#                           at all passes it.
#   no-display-diagnostic   the *GUI* path with neither WAYLAND_DISPLAY nor
#                           DISPLAY must print a diagnostic, not panic
#                           (D-12a / N-01). Exit 101 plus a winit
#                           `NotSupported` traceback is a regression against
#                           the Qt app.
#   gui-stays-up            under a compositor, the GUI starts and is still
#                           running after --hold seconds, prints no panic, and
#                           terminates on SIGTERM rather than hanging.
#
# Both CLI checks are *smoke* checks and are deliberately shallow: they prove the
# packaged binary starts and the verbs are reachable, nothing more. What the
# verbs actually print and return is gated by verify.sh's `cli` stage, against a
# library it owns -- a check that only exercises the empty library passes on a
# `--list` that never reads the file, which is exactly what happened (D-35,
# task #31). Do not treat these two lines as coverage of the CLI.
#
# Exit status:  0 = every check that could run passed.
#               1 = at least one check FAILED.
#              77 = only the CLI checks ran; no compositor was available, so
#                   `gui-stays-up` was SKIPPED. Reported loudly, never green.
#
# This script never fakes a pass: if the app crashes, `gui-stays-up` fails and
# the captured output is printed.
#
set -uo pipefail

APP_ID="com.goshapps.GameHandler"

usage() {
    cat <<'EOF'
usage: scripts/smoke-test.sh [options]

  --build-dir DIR     flatpak-builder output tree (default: <repo>/build-flatpak)
  --hold SECONDS      how long the GUI must stay alive (default: 8)
  --timeout SECONDS   grace period for SIGTERM (default: 20)
  --app-cmd "CMD"     run this command inside the sandbox instead of the app's
                      own binary. Word-split on spaces. Self-test hook:
                      `--app-cmd /app/bin/7z` must FAIL with
                      "exited ... before the hold interval", which is how the
                      failure path is proven to work rather than assumed.
  --compositor "CMD"  start this command as the headless display server instead
                      of auto-detecting one. The script waits for a socket to
                      appear before using it.
  --no-compositor     skip `gui-stays-up` even if a display is available.

Precedence for the display used by `gui-stays-up`: --compositor, then a
headless compositor on PATH (weston, Xvfb), then the ambient
WAYLAND_DISPLAY/DISPLAY session, then SKIP.
EOF
}

SELF="${BASH_SOURCE[0]}"
ROOT="$(cd "$(dirname "$SELF")/.." && pwd)"

BUILD_DIR="$ROOT/build-flatpak"
APP_CMD_STR="/app/bin/gamehandler"
HOLD=8
TERM_TIMEOUT=20
COMPOSITOR_CMD=""
ALLOW_COMPOSITOR=1

while [ $# -gt 0 ]; do
    case "$1" in
        --build-dir)     BUILD_DIR="${2:?--build-dir needs a value}"; shift 2 ;;
        --hold)          HOLD="${2:?--hold needs a value}"; shift 2 ;;
        --timeout)       TERM_TIMEOUT="${2:?--timeout needs a value}"; shift 2 ;;
        --app-cmd)       APP_CMD_STR="${2:?--app-cmd needs a value}"; shift 2 ;;
        --compositor)    COMPOSITOR_CMD="${2:?--compositor needs a value}"; shift 2 ;;
        --no-compositor) ALLOW_COMPOSITOR=0; shift ;;
        -h|--help)       usage; exit 0 ;;
        *)               echo "smoke-test.sh: unknown option: $1" >&2; exit 2 ;;
    esac
done

# Word-split the command. Deliberate: this is a human-facing test hook, and no
# documented invocation needs quoting inside it.
read -r -a APP_CMD <<<"$APP_CMD_STR"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/gh-smoke-XXXXXX")"
SPAWN_PID=""
APP_PID=""
FAILURES=0
SKIPPED=0

# ---------------------------------------------------------------- reporting

say()  { printf '%s\n' "$*"; }
ok()   { printf 'ok   %s\n' "$1"; }
bad()  { printf 'FAIL %s\n' "$1"; FAILURES=$((FAILURES + 1)); }
skip() { printf 'SKIP %s\n' "$1"; SKIPPED=$((SKIPPED + 1)); }

# Print the tail of a captured log, indented, so a failure is diagnosable from
# the output alone. An empty log says so rather than printing nothing. Bounded
# to DUMP_LINES so one noisy binary cannot bury the other checks.
DUMP_LINES=40
dump() {
    local label="$1" file="$2"
    local total
    total="$(wc -l <"$file" 2>/dev/null || echo 0)"
    say "     --- $label ---"
    if [ -s "$file" ]; then
        [ "$total" -gt "$DUMP_LINES" ] && say "     | (last $DUMP_LINES of $total lines)"
        tail -n "$DUMP_LINES" "$file" | sed 's/^/     | /'
    else
        say "     | (no output)"
    fi
    say "     --- end $label ---"
}

cleanup() {
    if [ -n "$APP_PID" ] && kill -0 "$APP_PID" 2>/dev/null; then
        kill -TERM "$APP_PID" 2>/dev/null
        for _ in $(seq 1 20); do kill -0 "$APP_PID" 2>/dev/null || break; sleep 0.25; done
        kill -KILL "$APP_PID" 2>/dev/null
    fi
    if [ -n "$SPAWN_PID" ] && kill -0 "$SPAWN_PID" 2>/dev/null; then
        kill -TERM "$SPAWN_PID" 2>/dev/null
        sleep 0.5
        kill -KILL "$SPAWN_PID" 2>/dev/null
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT

# ------------------------------------------------------------------- runner
#
# Two ways to reach the app, in descending order of fidelity:
#
#   installed  `flatpak run com.goshapps.GameHandler` — the real sandbox, built
#              from the manifest's own finish-args. Used when the app is
#              installed in the user installation.
#   build-dir  `flatpak build <dir> <cmd>` — runs the artifacts flatpak-builder
#              produced without installing anything. Sockets are *not*
#              inherited from the manifest here (flatpak-build(1) assembles its
#              sandbox from explicit flags instead), so the calls below pass
#              the two the app needs: wayland and fallback-x11.
#
# RUN_MODE is decided once, before any check.

RUN_MODE=""
if flatpak info "$APP_ID" >/dev/null 2>&1; then
    RUN_MODE="installed"
elif [ -d "$BUILD_DIR/files" ]; then
    RUN_MODE="build-dir"
fi

if [ -z "$RUN_MODE" ]; then
    say "smoke-test.sh: no way to run the app."
    say "  'flatpak info $APP_ID' found no installed app, and the build dir"
    say "  '$BUILD_DIR' does not exist."
    say "  Run flatpak-builder first (scripts/verify.sh stage 'flatpak-build'),"
    say "  or install the built repo with 'flatpak install --user flatpak-repo $APP_ID'."
    exit 1
fi

# Run the app once and return its exit status.
#
#   run_once LOG SOCKET ENVS [ARGS...]
#
# SOCKET is one of: none | wayland | x11. `none` additionally unsets the
# display variables inside the sandbox, so a check cannot accidentally pass
# because the caller's session leaked in. ENVS is a string of extra
# `flatpak build` arguments (empty for installed mode, which takes its
# sandbox from the manifest).
run_once() {
    local log="$1"; shift
    local sock="$1"; shift
    local envs="$1"; shift

    if [ "$RUN_MODE" = "installed" ]; then
        local -a cmd=(flatpak run --command="${APP_CMD[0]}")
        if [ "$sock" = "none" ]; then
            cmd+=(--unset-env=DISPLAY --unset-env=WAYLAND_DISPLAY --unset-env=WAYLAND_SOCKET)
        fi
        cmd+=("$APP_ID" "${APP_CMD[@]:1}" "$@")
        "${cmd[@]}" >"$log" 2>&1
    else
        local -a cmd=(flatpak build)
        case "$sock" in
            none)    cmd+=(--unset-env=DISPLAY --unset-env=WAYLAND_DISPLAY --unset-env=WAYLAND_SOCKET) ;;
            wayland) cmd+=(--socket=wayland --share=ipc) ;;
            x11)     cmd+=(--socket=fallback-x11 --share=ipc) ;;
        esac
        [ -n "$envs" ] && cmd+=($envs)
        cmd+=("$BUILD_DIR" "${APP_CMD[@]}" "$@")
        "${cmd[@]}" >"$log" 2>&1
    fi
}

# Launch the app detached (for the long-running GUI check) and echo its pid.
launch_detached() {
    local log="$1"; shift
    local envs="$1"; shift

    if [ "$RUN_MODE" = "installed" ]; then
        flatpak run --command="${APP_CMD[0]}" "$APP_ID" "${APP_CMD[@]:1}" >"$log" 2>&1 &
    else
        # --die-with-parent so tearing the wrapper down also tears down the
        # sandbox; without it a killed wrapper leaves the app running.
        # shellcheck disable=SC2086
        flatpak build --die-with-parent --socket=wayland --socket=fallback-x11 \
            --share=ipc --device=all $envs "$BUILD_DIR" "${APP_CMD[@]}" >"$log" 2>&1 &
    fi
    APP_PID=$!
    echo "$APP_PID"
}

# Panic signatures. A run that prints one of these is a crash whatever its
# exit code says.
PANIC_RE="panicked at|fatal runtime error|RUST_BACKTRACE=1"

say "smoke-test.sh: mode=$RUN_MODE app=${APP_CMD[*]} hold=${HOLD}s"
[ "$RUN_MODE" = "build-dir" ] && say "               build dir: $BUILD_DIR"
say ""

# ------------------------------------------------------- check: cli-version

rc=0
run_once "$WORK/version.log" none "" --version || rc=$?
if [ "$rc" -eq 0 ] && grep -qE '^GameHandler [0-9]+\.[0-9]+\.[0-9]+$' "$WORK/version.log"; then
    ok "cli-version ($(head -n1 "$WORK/version.log"))"
elif [ "$rc" -ne 0 ]; then
    bad "cli-version: exit $rc with no display"
    dump "stdout+stderr" "$WORK/version.log"
else
    bad "cli-version: unexpected output"
    dump "stdout+stderr" "$WORK/version.log"
fi

# ---------------------------------------------------------- check: cli-list

rc=0
run_once "$WORK/list.log" none "" --list || rc=$?
if [ "$rc" -eq 0 ]; then
    ok "cli-list-exits-0 (exit 0, no display; output not examined)"
else
    bad "cli-list-exits-0: exit $rc with no display"
    dump "stdout+stderr" "$WORK/list.log"
fi

# ------------------------------------------- check: no-display-diagnostic
#
# DECISIONS D-12a / parity items N-01 and N-02. With no display at all the GUI
# path must print an actionable message and exit non-zero without a traceback.
# iced panics with `Create event loop: NotSupported` (exit 101) unless the app
# catches it. Checked with no arguments -- the path that opens a window.

rc=0
run_once "$WORK/nodisplay.log" none "" || rc=$?
if grep -qE "$PANIC_RE" "$WORK/nodisplay.log"; then
    bad "no-display-diagnostic (D-12a/N-01): the GUI path panicked instead of diagnosing"
    dump "stdout+stderr" "$WORK/nodisplay.log"
elif [ "$rc" -eq 0 ]; then
    bad "no-display-diagnostic (D-12a/N-02): exit 0 with no display; expected a diagnostic and a non-zero exit"
    dump "stdout+stderr" "$WORK/nodisplay.log"
elif [ ! -s "$WORK/nodisplay.log" ]; then
    bad "no-display-diagnostic (D-12a/N-01): exit $rc in silence; expected an actionable message"
else
    ok "no-display-diagnostic (exit $rc, message printed)"
fi

# ------------------------------------------------------- check: gui-stays-up

# Resolve a display. Preference order, so a CI box does not depend on the
# developer's session and a developer's session does not get windows it did
# not ask for:
#   1. --compositor CMD   (explicit; started here, torn down here)
#   2. a headless compositor on PATH (weston, Xvfb)
#   3. the ambient WAYLAND_DISPLAY / DISPLAY session
#   4. nothing -> SKIP
DISPLAY_KIND=""     # wayland | x11
DISPLAY_ENVS=""     # extra `flatpak build` arguments
DISPLAY_NOTE=""
DISPLAY_TRIED=0     # set when a display was attempted and failed, so the
                    # outcome is FAIL (already recorded) and not also SKIP

# Start a compositor and wait for its socket. Sets DISPLAY_KIND/DISPLAY_ENVS
# and, for the headless case, XDG_RUNTIME_DIR so flatpak proxies the new socket
# rather than the session's.
start_compositor() {
    local cmd="$1"
    local runtime="$WORK/run"
    local wl="wayland-smoke"

    mkdir -p "$runtime"
    chmod 700 "$runtime"

    say "     starting compositor: $cmd"
    (
        export XDG_RUNTIME_DIR="$runtime"
        export WAYLAND_DISPLAY="$wl"
        export DISPLAY=":97"
        # shellcheck disable=SC2086
        exec $cmd
    ) >"$WORK/compositor.log" 2>&1 &
    SPAWN_PID=$!

    local waited=0
    while [ "$waited" -lt 60 ]; do
        if [ -S "$runtime/$wl" ]; then
            DISPLAY_KIND="wayland"
            DISPLAY_ENVS="--env=WAYLAND_DISPLAY=$wl"
            export XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$wl"
            return 0
        fi
        if [ -S "/tmp/.X11-unix/X97" ]; then
            DISPLAY_KIND="x11"
            DISPLAY_ENVS="--env=DISPLAY=:97"
            export DISPLAY=":97"
            return 0
        fi
        kill -0 "$SPAWN_PID" 2>/dev/null || break
        sleep 0.5
        waited=$((waited + 1))
    done

    bad "gui-stays-up: the compositor did not come up"
    dump "compositor log" "$WORK/compositor.log"
    kill -KILL "$SPAWN_PID" 2>/dev/null
    SPAWN_PID=""
    return 1
}

resolve_display() {
    [ "$ALLOW_COMPOSITOR" -eq 1 ] || return 1

    # A compositor was started on purpose and did not come up. That is a
    # FAILURE of this check, already recorded by start_compositor -- it must
    # not also be reported as a SKIP, or a broken CI setup would read as
    # "nothing to do here".
    if [ -n "$COMPOSITOR_CMD" ]; then
        DISPLAY_TRIED=1
        start_compositor "$COMPOSITOR_CMD" || return 1
        DISPLAY_NOTE="compositor: $COMPOSITOR_CMD"
        return 0
    fi

    if command -v weston >/dev/null 2>&1; then
        DISPLAY_TRIED=1
        start_compositor "weston --backend=headless --idle-time=0 --socket=wayland-smoke" \
            && DISPLAY_NOTE="compositor: weston --backend=headless (headless)" \
            && return 0
        return 1
    fi

    if command -v Xvfb >/dev/null 2>&1; then
        DISPLAY_TRIED=1
        start_compositor "Xvfb :97 -screen 0 1280x1024x24 -nolisten tcp" \
            && DISPLAY_NOTE="compositor: Xvfb :97 (headless)" \
            && return 0
        return 1
    fi

    local runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
    if [ -n "${WAYLAND_DISPLAY:-}" ] && [ -S "$runtime/$WAYLAND_DISPLAY" ]; then
        DISPLAY_KIND="wayland"
        DISPLAY_ENVS="--env=WAYLAND_DISPLAY=$WAYLAND_DISPLAY"
        DISPLAY_NOTE="ambient Wayland session ($WAYLAND_DISPLAY) -- a window will appear on your desktop"
        return 0
    fi

    if [ -n "${DISPLAY:-}" ]; then
        DISPLAY_KIND="x11"
        DISPLAY_ENVS="--env=DISPLAY=$DISPLAY"
        DISPLAY_NOTE="ambient X11 session ($DISPLAY) -- a window will appear on your desktop"
        return 0
    fi

    return 1
}

if resolve_display; then
    say "     $DISPLAY_NOTE"

    launch_detached "$WORK/gui.log" "$DISPLAY_ENVS" >/dev/null

    slept=0
    while [ "$slept" -lt "$HOLD" ]; do
        kill -0 "$APP_PID" 2>/dev/null || break
        sleep 1
        slept=$((slept + 1))
    done

    if ! kill -0 "$APP_PID" 2>/dev/null; then
        wait "$APP_PID"; rc=$?
        bad "gui-stays-up: exited after ${slept}s, before the ${HOLD}s hold interval (status $rc)"
        dump "stdout+stderr" "$WORK/gui.log"
        APP_PID=""
    elif grep -qE "$PANIC_RE" "$WORK/gui.log"; then
        bad "gui-stays-up: alive but printed a panic"
        dump "stdout+stderr" "$WORK/gui.log"
        kill -TERM "$APP_PID" 2>/dev/null; wait "$APP_PID" 2>/dev/null; APP_PID=""
    else
        # Alive for the whole interval. It must also not hang on the way out.
        kill -TERM "$APP_PID" 2>/dev/null
        gone=0
        checks=$((TERM_TIMEOUT * 4))
        for _ in $(seq 1 "$checks"); do
            kill -0 "$APP_PID" 2>/dev/null || { gone=1; break; }
            sleep 0.25
        done
        wait "$APP_PID" 2>/dev/null
        if [ "$gone" -eq 1 ]; then
            ok "gui-stays-up (alive ${HOLD}s over $DISPLAY_KIND, terminated on SIGTERM)"
            dump "stdout+stderr" "$WORK/gui.log"
        else
            bad "gui-stays-up: still running ${TERM_TIMEOUT}s after SIGTERM"
            dump "stdout+stderr" "$WORK/gui.log"
            kill -KILL "$APP_PID" 2>/dev/null
        fi
        APP_PID=""
    fi
elif [ "$DISPLAY_TRIED" -eq 1 ]; then
    : # start_compositor already recorded the FAIL; do not also report a SKIP
else
    skip "gui-stays-up: no display available"
    say "     No compositor socket, and no headless compositor on PATH. This"
    say "     check needs a display server and nothing else -- no GPU, no"
    say "     Vulkan loader, no /dev/dri (DECISIONS D-11, software rendering)."
    say "     Install weston or Xvfb (both are auto-detected), or point this"
    say "     script at a compositor with --compositor 'CMD'."
    say "     NOT a pass: the GUI has not been exercised."
fi

say ""
if [ "$FAILURES" -gt 0 ]; then
    say "smoke-test.sh: $FAILURES check(s) FAILED, $SKIPPED skipped"
    exit 1
fi
if [ "$SKIPPED" -gt 0 ]; then
    say "smoke-test.sh: all runnable checks passed, $SKIPPED skipped"
    exit 77
fi
say "smoke-test.sh: all checks passed"
exit 0
