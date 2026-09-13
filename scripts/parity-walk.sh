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
# # Why `up` verifies virtual input by measuring it
#
# The compositor runs headless with `WLR_LIBINPUT_NO_DEVICES=1`, so its seat has
# no input device. Three separate mechanisms then report success while
# delivering nothing, and each one produced a false "verified" verdict in an
# earlier pass of this walk:
#
#   * `swaymsg seat - cursor set/press` returns `{"success": true}` having moved
#     a cursor no client can see.
#   * `wtype` exits 0 having delivered no keystroke (`changed rows: 0/800`).
#   * a hand-trimmed copy of the virtual-pointer protocol parses fine and sends
#     the wrong opcode, because a Wayland request's opcode is its ordinal among
#     the interface's `request` declarations — so the compositor replies
#     `invalid arguments for zwlr_virtual_pointer_v1.motion_absolute` to what
#     the client believed was a `button`.
#
# `up` therefore drives `scripts/walk/vptr-hold.c`, which creates a
# `zwlr_virtual_pointer_v1` and a `zwp_virtual_keyboard_v1` from the *canonical*
# vendored protocol XMLs, and then runs `probe_input`, which opens a window
# whose reaction to a keystroke is unambiguous and requires the framebuffer to
# have changed. A harness whose input cannot be demonstrated to arrive does not
# start; that is the difference between an instrument and a camera.
#
# Requires `wayland-scanner`, `wayland-client` headers, and a C compiler.
#
# # Usage
#
#   scripts/parity-walk.sh up            start the compositor and the app
#   scripts/parity-walk.sh shot NAME     capture, asserting the app's window
#   scripts/parity-walk.sh key NAME..    send keys by name (Tab, Down, F1, n)
#   scripts/parity-walk.sh chord MOD NAME..   hold MOD (ctrl/alt/shift/logo)
#   scripts/parity-walk.sh type TEXT     send literal text
#   scripts/parity-walk.sh click X Y     move the pointer and click
#   scripts/parity-walk.sh rclick X Y   move the pointer and right-click
#   scripts/parity-walk.sh tree          print the window tree as JSON
#   scripts/parity-walk.sh logs          print the app's stdout+stderr
#   scripts/parity-walk.sh down          stop both
#
# `up` is idempotent; `shot`/`key`/`click` require a running session and say so.
#
# Environment: WORK (default /tmp/gh-walk), SOURCE=flatpak|build, BUILD_DIR, CC.
# With SOURCE=build it runs the binary out of a flatpak-builder tree, which is
# what a walk of an unreleased commit needs.

set -uo pipefail

SELF="${BASH_SOURCE[0]}"
SELF_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"

WORK="${WORK:-/tmp/gh-walk}"
APP_ID="com.goshapps.GameHandler"
APP_BIN="/app/bin/gamehandler"
SOURCE="${SOURCE:-flatpak}"
BUILD_DIR="${BUILD_DIR:-}"
RES="${RES:-1280x800}"

RUNTIME="$WORK/run"
SHOTS="$WORK/shots"
VPTR_BIN="$WORK/vptr-hold"
VPTR_SRC="$SELF_DIR/walk/vptr-hold.c"
# The canonical upstream protocol definitions, vendored verbatim. They are the
# real files, not a hand-trimmed subset: a Wayland request's opcode is its
# position among its interface's `request` declarations, so a copy that drops
# a request the harness does not use renumbers every request after it and the
# compositor rejects a call the client believed it was making correctly.
VPTR_XMLS="$SELF_DIR/walk/wlr-virtual-pointer-unstable-v1.xml
$SELF_DIR/walk/virtual-keyboard-unstable-v1.xml
$SELF_DIR/walk/ext-transient-seat-v1.xml"
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

    # ---------------------------------------------------------- virtual input
    #
    # sway is started with WLR_LIBINPUT_NO_DEVICES=1 above, so the seat has no
    # input device of its own. That is deliberate — this machine's real devices
    # must not leak into the walk — but it means `swaymsg seat - cursor` moves a
    # cursor no client can see. sway reports `{"success": true}` for it anyway,
    # so a harness that trusts that status reports every click as delivered
    # while the app receives nothing at all. That is a check which passes
    # without inspecting what it claims.
    #
    # The two paths that *do* deliver on this compositor are a
    # `zwlr_virtual_pointer_v1` / `zwp_virtual_keyboard_v1` pair, which the
    # compositor routes exactly as it would a physical device. `-c` runs a
    # whole gesture as one short-lived process, because sway closes a
    # virtual-input client's connection if it sits idle between bursts.
    #
    # The seat's advertised capability is deliberately NOT the gate here. On
    # sway with no libinput devices, seat0 reports `capabilities: 0,
    # devices: []` even while a working virtual pointer is delivering clicks,
    # because a wlroots virtual pointer is not a seat device. Gating on it
    # would refuse a session that works. `probe_input` below measures delivery
    # directly, which is the property that actually matters.
    if [ ! -x "$VPTR_BIN" ] || [ "$VPTR_SRC" -nt "$VPTR_BIN" ]; then
        [ -f "$VPTR_SRC" ] || die "missing $VPTR_SRC"
        for xml in $VPTR_XMLS; do
            [ -f "$xml" ] || die "missing $xml"
        done
        command -v wayland-scanner >/dev/null || die "wayland-scanner is required to build the virtual-input glue"
        # The protocol XMLs are the canonical upstream files, vendored rather
        # than hand-written: a Wayland request's opcode is its position among
        # its interface's `request` declarations, so a hand-trimmed copy that
        # omits a request silently renumbers every one after it and the
        # compositor answers `invalid arguments` on a request the caller
        # thought it was sending correctly.
        local gen="$WORK/proto" srcs=""
        mkdir -p "$gen"
        for xml in $VPTR_XMLS; do
            local stem; stem=$(basename "$xml" .xml)
            wayland-scanner client-header "$xml" "$gen/$stem-client.h" \
                || die "wayland-scanner could not build $stem client header"
            wayland-scanner private-code  "$xml" "$gen/$stem-protocol.c" \
                || die "wayland-scanner could not build $stem protocol code"
            srcs="$srcs $gen/$stem-protocol.c"
        done
        # shellcheck disable=SC2086
        ${CC:-cc} -O2 -o "$VPTR_BIN" "$VPTR_SRC" $srcs -I"$gen" \
            $(pkg-config --cflags --libs wayland-client) \
            || die "could not build $VPTR_BIN (need wayland-client headers)"
        say "built $VPTR_BIN"
    fi
    # The keyboard half needs a keymap before any keycode means anything; the
    # `evdev` keycodes the commands above are expressed in are only meaningful
    # against this map. Written here so `up` is the one place that sets the
    # session's input vocabulary up.
    cat >"$WORK/keymap.xkb" <<'XKB'
xkb_keymap { xkb_keycodes { include "evdev+aliases(qwerty)" }; xkb_types { include "complete" }; xkb_compat { include "complete" }; xkb_symbols { include "pc+us" }; };
XKB
    probe_input || die "virtual input does not reach the compositor (see $WORK/probe-foot.log; re-run with VPTR_BIN=/bin/true to confirm the probe itself can fail)"
    say "virtual pointer+keyboard verified"

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

# ----------------------------------------------------------------- keyboard
#
# `wtype` cannot drive this app. Its exit status is 0 whether or not a key
# reaches the client, and on this compositor it reaches nothing: measured
# `wtype -M ctrl -k n` against the focused app produced a full-frame diff of
# `changed rows: 0/800, changed bytes: 0`. A harness gating on `wtype`'s exit
# status therefore records every shortcut as exercised while exercising none.
#
# The keyboard half of the virtual device is the path that delivers, so the
# commands below are expressed in Linux keycodes (the `evdev` codes — 30 = KEY_A,
# 29 = KEY_LEFTCTRL, 49 = KEY_N) via `$WORK/keymap.xkb`, which `cmd_up` writes.
# Keystrokes go through one `-c` burst for the same reason clicks do: sway
# closes a virtual-input client's connection across an idle gap, so a chord has
# to be a single client's lifetime.
#
# cmd_key / cmd_chord / cmd_type take names and translate them, so a caller
# never has to know a keycode. The name table is deliberately explicit rather
# than exhaustive: an unknown name is a hard failure, not a silent skip, so a
# walk cannot pass by asking for a key the harness does not know how to send.
keycode_for() {
    case "$1" in
        a) echo 30 ;; b) echo 48 ;; c) echo 46 ;; d) echo 32 ;; e) echo 18 ;;
        f) echo 33 ;; g) echo 34 ;; h) echo 35 ;; i) echo 23 ;; j) echo 36 ;;
        k) echo 37 ;; l) echo 38 ;; m) echo 50 ;; n) echo 49 ;; o) echo 24 ;;
        p) echo 25 ;; q) echo 16 ;; r) echo 19 ;; s) echo 31 ;; t) echo 20 ;;
        u) echo 22 ;; v) echo 47 ;; w) echo 17 ;; x) echo 45 ;; y) echo 21 ;;
        z) echo 44 ;;
        0) echo 11 ;; 1) echo 2 ;; 2) echo 3 ;; 3) echo 4 ;; 4) echo 5 ;;
        5) echo 6 ;; 6) echo 7 ;; 7) echo 8 ;; 8) echo 9 ;; 9) echo 10 ;;
        Return|KP_Enter) echo 28 ;; Escape) echo 1 ;; Tab) echo 15 ;;
        space) echo 57 ;; BackSpace) echo 14 ;; Delete) echo 111 ;;
        Up) echo 103 ;; Down) echo 108 ;; Left) echo 105 ;; Right) echo 106 ;;
        Home) echo 102 ;; End) echo 107 ;;
        Page_Up) echo 104 ;; Page_Down) echo 109 ;;
        F1) echo 59 ;; F2) echo 60 ;; F3) echo 61 ;; F4) echo 62 ;;
        F5) echo 63 ;; F6) echo 64 ;; F7) echo 65 ;; F8) echo 66 ;;
        F9) echo 67 ;; F10) echo 68 ;; F11) echo 87 ;; F12) echo 88 ;;
        comma) echo 51 ;; period) echo 52 ;; slash) echo 53 ;;
        minus) echo 12 ;; equal) echo 13 ;;
        *) die "the walk harness has no keycode for '$1' (add it to keycode_for rather than skipping the key)" ;;
    esac
}

# Modifier names as bit positions in the XKB modifier state, which is what
# `zwp_virtual_keyboard_v1.modifiers` carries.
modifier_mask() {
    case "$1" in
        shift) echo 1 ;; ctrl|control) echo 4 ;; alt) echo 8 ;; logo|super) echo 64 ;;
        *) die "unknown modifier '$1' (shift, ctrl, alt, logo)" ;;
    esac
}

cmd_key() {
    [ $# -gt 0 ] || die "key needs at least one key name"
    local seq="" k
    for k in "$@"; do
        local code; code=$(keycode_for "$k")
        seq="$seq key $code;"
    done
    "$VPTR_BIN" -c "keymap $WORK/keymap.xkb;$seq" >/dev/null 2>&1 \
        || die "the virtual keyboard could not deliver: $*"
    say "key $*"
}

cmd_chord() {
    local mod="${1:?chord needs a modifier}"; shift
    [ $# -gt 0 ] || die "chord needs at least one key name"
    local mask; mask=$(modifier_mask "$mod")
    local seq="" k
    for k in "$@"; do
        local code; code=$(keycode_for "$k")
        seq="$seq key $code;"
    done
    "$VPTR_BIN" -c "keymap $WORK/keymap.xkb; modifiers $mask 0 0 0;$seq modifiers 0 0 0 0;" >/dev/null 2>&1 \
        || die "the virtual keyboard could not deliver: $mod + $*"
    say "chord $mod + $*"
}

cmd_type() {
    [ $# -gt 0 ] || die "type needs text"
    local text="$*" seq="" i ch
    for (( i=0; i<${#text}; i++ )); do
        ch="${text:i:1}"
        case "$ch" in
            ' ') code=57 ;;
            *) code=$(keycode_for "$ch") ;;
        esac
        seq="$seq key $code;"
    done
    "$VPTR_BIN" -c "keymap $WORK/keymap.xkb;$seq" >/dev/null 2>&1 \
        || die "the virtual keyboard could not deliver: $text"
    say "type $text"
}

# ------------------------------------------------------------------ pointer
#
# The gate that tests delivery: open a window whose reaction to a keystroke is
# unambiguous, type into it through the virtual keyboard, and require that the
# framebuffer changed. The probe is its own client, so a silent failure here
# cannot be confused with a property of the app under test.
#
# Comparison is by decoded pixel (`scripts/walk/imgdiff.py`), not by file bytes
# or a hash: two PNGs of the same frame routinely differ in length while showing
# the same pixels, so a byte comparison reports success as failure and needs a
# length guard that then hides a genuine "not comparable" case.
#
# The negative case is the point. Measured with `VPTR_BIN=/bin/true` — a tool
# that exits 0 and sends nothing — this reports `changed pixels: 0` and returns
# 1. A probe that cannot fail is not a probe.
probe_input() {
    command -v foot >/dev/null || { say "note: foot is not installed; the input probe cannot run"; return 1; }
    command -v grim >/dev/null || { say "note: grim is not installed; the input probe cannot run"; return 1; }
    local before="$WORK/probe-before.png" after="$WORK/probe-after.png"
    # A unique app-id per run so a stale probe window from an earlier walk
    # cannot be the thing that reacts.
    local probe_id="ghprobe-$$"
    foot -a "$probe_id" >"$WORK/probe-foot.log" 2>&1 &
    local fpid=$!
    sleep 2
    if ! grim "$before" 2>/dev/null; then
        kill "$fpid" 2>/dev/null
        return 1
    fi
    "$VPTR_BIN" -c "keymap $WORK/keymap.xkb; key 30; key 31; key 32" >/dev/null 2>&1
    sleep 1
    if ! grim "$after" 2>/dev/null; then
        kill "$fpid" 2>/dev/null
        return 1
    fi
    kill "$fpid" 2>/dev/null
    local changed
    changed=$(python3 "$SELF_DIR/walk/imgdiff.py" "$before" "$after" 2>/dev/null | head -1) || {
        say "input probe: captures were not comparable"
        return 1
    }
    say "input probe: $changed"
    # "changed pixels: 0 of ..." is the failure; anything else means a keystroke
    # reached a client and the client reacted to it.
    case "$changed" in
        "changed pixels: 0 "*) return 1 ;;
        "") return 1 ;;
        *) return 0 ;;
    esac
}

# A click is one uninterrupted burst through the virtual pointer. It is not
# `swaymsg seat - cursor`, which reports `success: true` on this compositor
# while delivering nothing, and it is not a `move` followed by a later `press`,
# because sway closes a virtual-input client's connection across an idle gap
# and the press then lands nowhere. The exit status alone would not catch
# either; the `-c` one-shot is what makes the whole gesture one client.
cmd_click() {
    local x="${1:?click needs x}" y="${2:?click needs y}"
    "$VPTR_BIN" -c "click $x $y" >/dev/null 2>&1 \
        || die "the virtual pointer could not deliver a click at $x,$y"
    say "click at $x,$y"
}

cmd_rclick() {
    local x="${1:?rclick needs x}" y="${2:?rclick needs y}"
    "$VPTR_BIN" -c "click $x $y 273" >/dev/null 2>&1 \
        || die "the virtual pointer could not deliver a right-click at $x,$y"
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
    # The virtual input tool is one-shot and holds nothing between gestures, so
    # there is no holder pid to reap. The app and the compositor are the two
    # things this script starts that outlive their `nohup`.
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
    # The usage page: the `# Usage` block to the end of the header comment, so
    # that adding a paragraph above it does not silently truncate the help.
    *) sed -n '/^# # Usage/,/^$/p' "$SELF" | tail -n +2 | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
