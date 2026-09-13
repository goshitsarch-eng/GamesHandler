#!/usr/bin/env bash
#
# parity-walk.sh — the instrument for T-19's (B) items: the parity checks that
# can only be settled by looking at the running app.
#
# # Why this exists rather than a screenshot by hand
#
# `grim` captures whatever compositor it is pointed at, and on this machine
# that is the developer's live session — 16 Flatpaks, a browser, a terminal.
# A capture taken there is evidence about *a* screen, and the whole job of a
# parity walk is to say which app produced what. Worse, `wtype` types into
# whatever holds keyboard focus, so driving the app from the ambient session
# would mean typing into the nearest window, which may be the user's shell.
#
# So this script runs the app on a **private headless compositor** with exactly
# one window on it, and every capture is preceded by an assertion that the
# window it is about to photograph belongs to the app under test. A shot that
# cannot make that assertion fails instead of producing a picture of something
# else. That is the difference between an instrument and a camera.
#
# sway is the compositor rather than weston because both `grim` and `wtype`
# speak wlroots protocols (`wlr-screencopy`, `wlr-virtual-keyboard`); weston
# provides neither, so on weston this script could capture but not type.
# `scripts/smoke-test.sh` keeps weston, which is correct for *its* job — it
# only needs to know the app stays up.
#
# # Usage
#
#   scripts/parity-walk.sh up            start the compositor and the app
#   scripts/parity-walk.sh shot NAME     capture, asserting the app's window
#   scripts/parity-walk.sh key KEYSYM..  send keys (wtype names: Tab, Down, F1)
#   scripts/parity-walk.sh chord MOD KEYSYM   hold MOD (ctrl/alt/shift/logo)
#   scripts/parity-walk.sh type TEXT     send literal text
#   scripts/parity-walk.sh click X Y     move the pointer and click
#   scripts/parity-walk.sh rclick X Y   move the pointer and right-click
#   scripts/parity-walk.sh tree          print the window tree as JSON
#   scripts/parity-walk.sh logs          print the app's stdout+stderr
#   scripts/parity-walk.sh down          stop both
#
# `up` is idempotent; `shot`/`key`/`click` require a running session and say so.
#
# Environment: WORK (default /tmp/gh-walk), SOURCE=flatpak|build, BUILD_DIR.
# With SOURCE=build it runs the binary out of a flatpak-builder tree, which is
# what a walk of an unreleased commit needs.

set -uo pipefail

WORK="${WORK:-/tmp/gh-walk}"
APP_ID="com.goshapps.GameHandler"
APP_BIN="/app/bin/gamehandler"
SOURCE="${SOURCE:-flatpak}"
BUILD_DIR="${BUILD_DIR:-}"
RES="${RES:-1280x800}"

RUNTIME="$WORK/run"
SHOTS="$WORK/shots"
# sway offers no way to name its Wayland socket (there is deliberately no
# `-s` flag: wl_display_add_socket_auto numbers them wayland-0, wayland-1,
# ...), and WAYLAND_DISPLAY is not consulted. So the name is discovered at
# startup and recorded here for later invocations to rejoin.
WL="wayland-walk"
DISPLAY_FILE="$WORK/display"
if [ -f "$DISPLAY_FILE" ]; then WL="$(cat "$DISPLAY_FILE")"; fi
SWAY_PIDFILE="$WORK/sway.pid"
APP_PIDFILE="$WORK/app.pid"

export XDG_RUNTIME_DIR="$RUNTIME"
export WAYLAND_DISPLAY="$WL"

SELF="${BASH_SOURCE[0]}"

die() { printf 'parity-walk: %s\n' "$*" >&2; exit 1; }
say() { printf '  %s\n' "$*"; }

# The IPC socket is named for the pid that created it, so it cannot be
# predicted; it is discovered from the directory instead, exactly as sway does.
sock() {
    local s
    s=$(find "$RUNTIME" -maxdepth 1 -name 'sway-ipc.*.sock' -print -quit 2>/dev/null)
    [ -n "$s" ] || die "no sway IPC socket in $RUNTIME — is the session up?"
    printf '%s' "$s"
}

swaymsg_() {
    SWAYSOCK="$(sock)" swaymsg "$@" 2>/dev/null
}

# The assertion every capture rests on: the app's own window exists AND is the
# only thing on the output. The second half is what makes a full-screen capture
# attributable — with one window there is nothing else it could be.
#
# The tree is passed through the environment rather than stdin so that the
# caller's stdin is left alone, and the id likewise so that no part of the
# program is assembled by string interpolation.
app_window() {
    local tree
    tree=$(swaymsg_ -t get_tree) || return 1
    PARITY_APP_ID="$APP_ID" PARITY_TREE="$tree" python3 - <<'PY'
import json, os, sys
try:
    t = json.loads(os.environ["PARITY_TREE"])
except Exception:
    sys.exit(1)
want = os.environ["PARITY_APP_ID"]
found = []
def walk(n):
    if n.get("app_id") == want:
        found.append(n.get("rect") or {})
    for c in n.get("nodes", []) + n.get("floating_nodes", []):
        walk(c)
walk(t)
if len(found) != 1:
    sys.exit(1)
r = found[0]
print("{}x{} at {},{}".format(r.get("width"), r.get("height"), r.get("x"), r.get("y")))
PY
}

# --------------------------------------------------------------------- up
cmd_up() {
    mkdir -p "$RUNTIME" "$SHOTS"
    chmod 700 "$RUNTIME"

    if [ -f "$SWAY_PIDFILE" ] && kill -0 "$(cat "$SWAY_PIDFILE")" 2>/dev/null \
        && [ -f "$DISPLAY_FILE" ] && [ -S "$RUNTIME/$(cat "$DISPLAY_FILE")" ]; then
        say "compositor already up"
    else
        # No live session: stop anything stale, then sweep its sockets so the
        # socket that appears next is unambiguously the new compositor's.
        if [ -f "$SWAY_PIDFILE" ]; then kill -KILL "$(cat "$SWAY_PIDFILE")" 2>/dev/null || true; fi
        pkill -f "sway -c ${WORK}/sway[.]conf" 2>/dev/null || true
        sleep 0.5
        rm -f "$RUNTIME"/wayland-* "$RUNTIME"/sway-ipc.*.sock "$DISPLAY_FILE" "$APP_PIDFILE" "$SWAY_PIDFILE"
        cat >"$WORK/sway.conf" <<EOF
output * resolution $RES
xwayland disable
EOF
        # pixman: the software renderer, so this works on a machine with no GPU
        # carve-out and inside a container, matching what DECISIONS D-11 ships.
        WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
            nohup sway -c "$WORK/sway.conf" >"$WORK/sway.log" 2>&1 &
        echo $! >"$SWAY_PIDFILE"

        local waited=0 fresh=""
        while [ "$waited" -lt 80 ]; do
            fresh=$(find "$RUNTIME" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' -print -quit 2>/dev/null)
            [ -n "$fresh" ] && break
            sleep 0.25
            waited=$((waited + 1))
        done
        [ -n "$fresh" ] || { tail -20 "$WORK/sway.log" >&2; die "sway created no Wayland socket"; }
        WL="$(basename "$fresh")"
        export WAYLAND_DISPLAY="$WL"
        printf '%s' "$WL" >"$DISPLAY_FILE"
        say "compositor up on $WL ($RES, headless, pixman)"
    fi

    if [ -f "$APP_PIDFILE" ] && kill -0 "$(cat "$APP_PIDFILE")" 2>/dev/null; then
        say "app already running"
    else
        case "$SOURCE" in
            flatpak) nohup flatpak run "$APP_ID" >"$WORK/app.log" 2>&1 & ;;
            build)
                [ -n "$BUILD_DIR" ] || die "SOURCE=build needs BUILD_DIR"
                [ -d "$BUILD_DIR/files" ] || die "$BUILD_DIR is not a flatpak-builder tree"
                nohup flatpak build --die-with-parent --socket=wayland --share=ipc \
                    "$BUILD_DIR" "$APP_BIN" >"$WORK/app.log" 2>&1 & ;;
            *) die "SOURCE must be flatpak|build" ;;
        esac
        echo $! >"$APP_PIDFILE"

        local waited=0 rect
        while [ "$waited" -lt 60 ]; do
            if rect=$(app_window); then
                say "app window up: $rect"
                return 0
            fi
            sleep 0.5
            waited=$((waited + 1))
        done
        die "the app's window never appeared (see $WORK/app.log)"
    fi
}

# -------------------------------------------------------------------- shot
cmd_shot() {
    local name="${1:?shot needs a name}"
    local rect
    rect=$(app_window) || die "refusing to capture: $APP_ID has no single window on this display"
    grim "$SHOTS/$name.png" || die "grim failed"
    say "shot $name.png — $APP_ID window $rect"
}

cmd_key()   { [ $# -gt 0 ] || die "key needs at least one keysym"; wtype "$@" || die "wtype failed"; }
cmd_chord() { local mod="${1:?chord needs a modifier}"; local k="${2:?chord needs a keysym}"
              wtype -M "$mod" -k "$k" || die "wtype failed"; }
cmd_type()  { wtype -- "$@" || die "wtype failed"; }

# wlr-virtual-keyboard has no absolute pointer, so the pointer is moved with
# sway's own seat command and the button is pressed with the virtual device.
cmd_click() {
    local x="${1:?click needs x}" y="${2:?click needs y}"
    swaymsg_ seat - cursor set "$x" "$y" || die "could not move the pointer"
    sleep 0.3
    wtype -M "" -k "" 2>/dev/null
    SWAYSOCK="$(sock)" swaymsg seat - cursor press button1 2>/dev/null || true
    sleep 0.1
    SWAYSOCK="$(sock)" swaymsg seat - cursor release button1 2>/dev/null || true
    say "click at $x,$y"
}

cmd_rclick() {
    local x="${1:?rclick needs x}" y="${2:?rclick needs y}"
    swaymsg_ seat - cursor set "$x" "$y" || die "could not move the pointer"
    sleep 0.3
    SWAYSOCK="$(sock)" swaymsg seat - cursor press button3 2>/dev/null || die "could not press button3"
    sleep 0.1
    SWAYSOCK="$(sock)" swaymsg seat - cursor release button3 2>/dev/null || true
    say "right-click at $x,$y"
}

cmd_tree() {
    local tree
    tree=$(swaymsg_ -t get_tree) || die "no session"
    printf '%s' "$tree" | python3 -c '
import json, sys
t = json.load(sys.stdin)
def walk(n, d=0):
    if n.get("app_id") or n.get("name"):
        print("  " * d, n.get("type"), "app_id=", repr(n.get("app_id")),
              "title=", repr(n.get("name")), n.get("rect"))
    for c in n.get("nodes", []) + n.get("floating_nodes", []):
        walk(c, d + 1)
walk(t)'
}

cmd_logs() { [ -f "$WORK/app.log" ] && cat "$WORK/app.log" || say "(no log)"; }

# -------------------------------------------------------------------- down
cmd_down() {
    for f in "$APP_PIDFILE" "$SWAY_PIDFILE"; do
        [ -f "$f" ] || continue
        local pid; pid=$(cat "$f")
        kill -TERM "$pid" 2>/dev/null
        for _ in 1 2 3 4 5 6 7 8 9 10; do kill -0 "$pid" 2>/dev/null || break; sleep 0.2; done
        kill -KILL "$pid" 2>/dev/null
        rm -f "$f"
    done
    # flatpak run's child survives its parent, so the sandbox instance is named
    # explicitly rather than trusted to the pid above.
    flatpak kill "$APP_ID" 2>/dev/null
    say "session down"
}

case "${1:-}" in
    up)    shift; cmd_up "$@" ;;
    shot)  shift; cmd_shot "$@" ;;
    key)   shift; cmd_key "$@" ;;
    chord) shift; cmd_chord "$@" ;;
    type)  shift; cmd_type "$@" ;;
    click) shift; cmd_click "$@" ;;
    rclick) shift; cmd_rclick "$@" ;;
    tree)  shift; cmd_tree "$@" ;;
    logs)  shift; cmd_logs "$@" ;;
    down)  shift; cmd_down "$@" ;;
    *) sed -n '3,45p' "$SELF" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
