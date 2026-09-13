// vptr-hold: a real virtual pointer and keyboard for the headless walk.
//
// Why: on a headless sway with no input devices, `swaymsg seat - cursor
// set/press` reports success while delivering nothing to the client — the
// seat has no pointer resource and sway's cursor is not the input path the
// compositor delivers from. This process creates a zwlr_virtual_pointer_v1
// and a zwp_virtual_keyboard_v1 and then stays in the foreground holding
// them, so the seat keeps both capabilities, and drives them from commands
// on stdin. The compositor routes what it sends exactly as it would a
// physical device, which is what makes a click in the walk mean something.
//
// Commands (one per line on stdin; replies are printed as "# <command>")
//   click X Y [BTN]  the whole action in one burst: home, move, press, release
//   home           park at the output origin (makes `move` absolute for real)
//   move  X Y      absolute move to X,Y on the current output
//   moveby DX DY   relative move
//   press N        button N down (272 = BTN_LEFT)
//   release N      button N up
//   key N          press+release linux keycode N
//   keydown/keyup N
//   keymap FILE    install an XKB keymap from FILE (XKB_V1)
//   quit
//
// `-c 'cmd; cmd; ...'` runs that one-shot sequence and exits, which is the
// shape the harness uses: sway closes a virtual-pointer client's connection
// if it stays idle between bursts of motion, so a click is sent as a single
// short-lived process rather than by driving a long-lived one across seconds.
//
// `move` needs the current pointer position to convert an absolute target
// into the relative motion the protocol carries, and nothing tells this
// program where the pointer is. It therefore *tracks* the position from its
// own moves, which is only sound after `home` has put the pointer somewhere
// known. Run `home` once at the start of a one-shot sequence.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <errno.h>
#include <poll.h>
#include <unistd.h>
#include <fcntl.h>
#include <wayland-client.h>
#include "wlr-virtual-pointer-unstable-v1-client.h"
#include "virtual-keyboard-unstable-v1-client.h"
#include "ext-transient-seat-v1-client.h"

#define BTN_LEFT 272

#define MAX_OUTPUTS 8

static struct wl_display *disp = NULL;
static struct wl_seat *seat = NULL;
static struct zwlr_virtual_pointer_manager_v1 *mgr = NULL;
static struct zwp_virtual_keyboard_manager_v1 *kmgr = NULL;
static struct ext_transient_seat_manager_v1 *tseat_mgr = NULL;
static struct zwlr_virtual_pointer_v1 *vptr = NULL;
static struct zwp_virtual_keyboard_v1 *vkbd = NULL;

static struct wl_output *outputs[MAX_OUTPUTS];
static int output_width[MAX_OUTPUTS];
static int output_height[MAX_OUTPUTS];
static int output_count = 0;

static double cursor_x = 0.0, cursor_y = 0.0;

static uint32_t now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint32_t)((ts.tv_sec * 1000 + ts.tv_nsec / 1000000) & 0xFFFFFFFF);
}

static void output_geometry(void *d, struct wl_output *o, int32_t x, int32_t y,
                            int32_t pw, int32_t ph, int32_t subpixel,
                            const char *make, const char *model,
                            int32_t transform) {
    (void)d; (void)o; (void)x; (void)y; (void)pw; (void)ph; (void)subpixel;
    (void)make; (void)model; (void)transform;
}

static void output_mode(void *d, struct wl_output *o, uint32_t flags,
                        int32_t width, int32_t height, int32_t refresh) {
    (void)d; (void)refresh;
    if (!(flags & WL_OUTPUT_MODE_CURRENT)) return;
    for (int i = 0; i < output_count; i++) {
        if (outputs[i] == o) {
            output_width[i] = width;
            output_height[i] = height;
            return;
        }
    }
}

static void output_done(void *d, struct wl_output *o) { (void)d; (void)o; }
static void output_scale(void *d, struct wl_output *o, int32_t f) {
    (void)d; (void)o; (void)f;
}

static const struct wl_output_listener output_listener = {
    output_geometry, output_mode, output_done, output_scale, NULL,
};

// The seat a transient seat hands back lives in the registry under a global
// name the `ready` event carries: the client binds that name to get the
// wl_seat. `transient_global` records the name so the registry handler binds
// *that* seat as the transient one and leaves the compositor's advertised
// seat as the fallback.
static uint32_t transient_global = 0;
static struct wl_seat *transient_seat = NULL;

static void transient_ready(void *d, struct ext_transient_seat_v1 *ts,
                            uint32_t global_name) {
    (void)d; (void)ts;
    transient_global = global_name;
}

static void transient_denied(void *d, struct ext_transient_seat_v1 *ts) {
    (void)d; (void)ts;
    fprintf(stderr, "vptr-hold: transient seat denied by compositor\n");
}

static const struct ext_transient_seat_v1_listener transient_listener = {
    transient_ready,
    transient_denied,
};

static void registry_global(void *d, struct wl_registry *r, uint32_t name,
                            const char *iface, uint32_t ver) {
    (void)d; (void)ver;
    if (!strcmp(iface, wl_seat_interface.name)) {
        if (transient_global && name == transient_global) {
            transient_seat = wl_registry_bind(r, name, &wl_seat_interface, 1);
        } else {
            seat = wl_registry_bind(r, name, &wl_seat_interface, 1);
        }
    } else if (!strcmp(iface, wl_output_interface.name)) {
        if (output_count < MAX_OUTPUTS) {
            outputs[output_count] =
                wl_registry_bind(r, name, &wl_output_interface, 2);
            output_width[output_count] = 0;
            output_height[output_count] = 0;
            wl_output_add_listener(outputs[output_count], &output_listener, NULL);
            output_count++;
        }
    } else if (!strcmp(iface, zwlr_virtual_pointer_manager_v1_interface.name)) {
        mgr = wl_registry_bind(r, name,
                               &zwlr_virtual_pointer_manager_v1_interface, 1);
    } else if (!strcmp(iface, zwp_virtual_keyboard_manager_v1_interface.name)) {
        kmgr = wl_registry_bind(r, name,
                                &zwp_virtual_keyboard_manager_v1_interface, 1);
    } else if (!strcmp(iface, ext_transient_seat_manager_v1_interface.name)) {
        tseat_mgr = wl_registry_bind(r, name,
                                     &ext_transient_seat_manager_v1_interface, 1);
    }
}

static void registry_remove(void *d, struct wl_registry *r, uint32_t name) {
    (void)d; (void)r; (void)name;
}

static const struct wl_registry_listener reg_listener = {
    registry_global, registry_remove,
};

// wl_display_flush returns -1 with errno EAGAIN when the socket is full, which
// is normal backpressure and not a failure; the correct response is to wait
// for writability and try again. After writing, anything the compositor has
// sent us is read — a client that never reads its socket never sees the
// protocol error the compositor sent before closing it, and reports the
// resulting EPIPE instead of the actual fault. Anything else is fatal, and the
// log handler below is what puts the compositor's own message on stderr.
static void log_handler(const char *fmt, va_list ap) {
    fputs("wayland: ", stderr);
    vfprintf(stderr, fmt, ap);
}

static int pump(void) {
    for (;;) {
        if (wl_display_flush(disp) == 0) break;
        if (errno != EAGAIN) return -1;
        struct pollfd pfd = { wl_display_get_fd(disp), POLLOUT, 0 };
        if (poll(&pfd, 1, 2000) <= 0) return -1;
    }
    struct pollfd pfd = { wl_display_get_fd(disp), POLLIN, 0 };
    while (poll(&pfd, 1, 0) > 0 && (pfd.revents & POLLIN)) {
        if (wl_display_dispatch(disp) < 0) return -1;
        pfd.revents = 0;
    }
    return 0;
}

// Move by a delta in one go. The protocol carries wl_fixed_t, so the value is
// scaled by 256; a delta larger than the output is split because a single
// oversized motion is clamped by the compositor at the output edge and the
// remainder is lost.
static void motion_by(double dx, double dy) {
    zwlr_virtual_pointer_v1_motion(vptr, now_ms(),
                                   wl_fixed_from_double(dx),
                                   wl_fixed_from_double(dy));
    zwlr_virtual_pointer_v1_frame(vptr);
}

static void clamp_tracking(void) {
    int w = output_width[0] > 0 ? output_width[0] : 1280;
    int h = output_height[0] > 0 ? output_height[0] : 800;
    if (cursor_x < 0) cursor_x = 0;
    if (cursor_y < 0) cursor_y = 0;
    if (cursor_x > w) cursor_x = w;
    if (cursor_y > h) cursor_y = h;
}

// An absolute move, expressed as the relative motion the protocol carries.
// Anything larger than the output is split, because a single oversized motion
// is clamped by the compositor at the output edge and the remainder is lost.
static void move_to(double x, double y) {
    double dx = x - cursor_x, dy = y - cursor_y;
    int w = output_width[0] > 0 ? output_width[0] : 1280;
    int h = output_height[0] > 0 ? output_height[0] : 800;
    while (dx != 0 || dy != 0) {
        double sx = dx > w ? (double)w : (dx < -w ? -(double)w : dx);
        double sy = dy > h ? (double)h : (dy < -h ? -(double)h : dy);
        motion_by(sx, sy);
        dx -= sx; dy -= sy;
    }
    cursor_x = x; cursor_y = y;
    clamp_tracking();
}

// Park at the output origin by sweeping well past the top-left corner, so the
// compositor's own clamping — not an assumption about where the pointer
// started — is what puts it there.
static void go_home(void) {
    for (int i = 0; i < 20; i++) motion_by(-128.0, -128.0);
    cursor_x = 0.0; cursor_y = 0.0;
}


// Whether to attach the virtual devices to the seat or create them globally.
//
// On this harness the advertised seat0 has no devices: sway runs with
// WLR_LIBINPUT_NO_DEVICES=1, so `swaymsg -t get_seats` reports
// `capabilities: 0, devices: []`. A virtual pointer attached to a seat that
// reports no pointer capability is accepted by the compositor and then
// delivers nothing — every click is a silent no-op that looks like a
// successful protocol exchange. Creating the device attached to the seat to
// which the compositor routes input (`-s`), or globally with a null seat
// (`-g`), are the two paths that can actually deliver.
static int use_global = 0;

// Connect, bind the seat and both managers, and create one virtual pointer and
// one virtual keyboard. Returns 0 on success.
static int setup(void) {
    wl_log_set_handler_client(log_handler);
    disp = wl_display_connect(NULL);
    if (!disp) { fprintf(stderr, "no display\n"); return 1; }
    struct wl_registry *reg = wl_display_get_registry(disp);
    wl_registry_add_listener(reg, &reg_listener, NULL);
    wl_display_roundtrip(disp);
    wl_display_roundtrip(disp);
    if (!mgr || !kmgr) {
        fprintf(stderr, "missing virtual pointer/keyboard manager\n");
        return 1;
    }
    struct wl_seat *target = use_global ? NULL : seat;

    // `-t`: ask the compositor for a transient seat. On a compositor whose own
    // seat advertises no capability, the virtual device has to be attached to a
    // seat the compositor will actually route from, and this is the protocol
    // for obtaining one. It has to happen in two steps because `ready` carries
    // the registry name of a *new* wl_seat global, which the registry handler
    // above then binds.
    if (tseat_mgr) {
        struct ext_transient_seat_v1 *ts =
            ext_transient_seat_manager_v1_create(tseat_mgr);
        if (ts) {
            ext_transient_seat_v1_add_listener(ts, &transient_listener, NULL);
            for (int i = 0; i < 20 && !transient_seat; i++) {
                if (wl_display_roundtrip(disp) < 0) break;
            }
        }
        if (transient_seat) {
            fprintf(stderr, "vptr-hold: transient seat obtained\n");
            target = transient_seat;
        } else {
            fprintf(stderr, "vptr-hold: no transient seat, using advertised seat\n");
        }
    }

    if (!target && !use_global) {
        fprintf(stderr, "vptr-hold: no seat to attach to\n");
        return 1;
    }

    vptr = zwlr_virtual_pointer_manager_v1_create_virtual_pointer(mgr, target);
    if (!vptr) { fprintf(stderr, "create virtual pointer failed\n"); return 1; }
    vkbd = zwp_virtual_keyboard_manager_v1_create_virtual_keyboard(kmgr, target);
    if (!vkbd) { fprintf(stderr, "create virtual keyboard failed\n"); return 1; }
    wl_display_roundtrip(disp);

    fprintf(stderr, "vptr-hold: %s pointer+keyboard held\n",
            use_global ? "global" : (transient_seat ? "transient-seat" : "seat-attached"));
    fflush(stderr);
    return 0;
}

// Run one command line. Returns 0 to continue, 1 to stop (quit), and -1 when
// the display connection is gone.
static int run_command(const char *line) {
        char cmd[64] = {0};
        double a = 0, b = 0;
        unsigned long n = 0;
        char path[256] = {0};
        int nf = sscanf(line, "%63s", cmd);
        if (nf != 1) return 0;

        if (!strcmp(cmd, "quit")) return 1;

        if (!strcmp(cmd, "click")) {
            // The whole action in one command: sway closes a virtual-input
            // client's connection between separate commands, so a click must
            // be one uninterrupted burst of requests rather than a `move`
            // followed by a later `press`.
            int bx = 0, by = 0;
            unsigned long btn = BTN_LEFT;
            if (sscanf(line, "%*s %d %d %lu", &bx, &by, &btn) < 2) {
                fprintf(stderr, "click needs X Y\n");
                printf("# click FAILED\n");
                fflush(stdout);
                return 0;
            }
            go_home();
            move_to((double)bx, (double)by);
            zwlr_virtual_pointer_v1_button(vptr, now_ms(), (uint32_t)btn,
                                           WL_POINTER_BUTTON_STATE_PRESSED);
            zwlr_virtual_pointer_v1_frame(vptr);
            if (pump() < 0) return -1;
            // A press and release in the same millisecond is one input event
            // as far as a toolkit's gesture recognition is concerned; the
            // settle is what makes it a click.
            usleep(80 * 1000);
            zwlr_virtual_pointer_v1_button(vptr, now_ms(), (uint32_t)btn,
                                           WL_POINTER_BUTTON_STATE_RELEASED);
            zwlr_virtual_pointer_v1_frame(vptr);
        } else if (!strcmp(cmd, "hover")) {
            int bx = 0, by = 0;
            if (sscanf(line, "%*s %d %d", &bx, &by) < 2) {
                fprintf(stderr, "hover needs X Y\n");
                printf("# hover FAILED\n");
                fflush(stdout);
                return 0;
            }
            go_home();
            move_to((double)bx, (double)by);
        } else if (!strcmp(cmd, "home")) {
            go_home();
        } else if (!strcmp(cmd, "move")) {
            sscanf(line, "%*s %lf %lf", &a, &b);
            move_to(a, b);
        } else if (!strcmp(cmd, "moveby")) {
            sscanf(line, "%*s %lf %lf", &a, &b);
            motion_by(a, b);
            cursor_x += a; cursor_y += b;
            clamp_tracking();
        } else if (!strcmp(cmd, "press") || !strcmp(cmd, "release")) {
            sscanf(line, "%*s %lu", &n);
            zwlr_virtual_pointer_v1_button(
                vptr, now_ms(), (uint32_t)n,
                strcmp(cmd, "press") ? WL_POINTER_BUTTON_STATE_RELEASED
                                     : WL_POINTER_BUTTON_STATE_PRESSED);
            zwlr_virtual_pointer_v1_frame(vptr);
        } else if (!strcmp(cmd, "key") || !strcmp(cmd, "keydown")
                   || !strcmp(cmd, "keyup")) {
            sscanf(line, "%*s %lu", &n);
            if (!strcmp(cmd, "key") || !strcmp(cmd, "keydown")) {
                zwp_virtual_keyboard_v1_key(vkbd, now_ms(), (uint32_t)n,
                                            WL_KEYBOARD_KEY_STATE_PRESSED);
            }
            if (!strcmp(cmd, "key") || !strcmp(cmd, "keyup")) {
                zwp_virtual_keyboard_v1_key(vkbd, now_ms(), (uint32_t)n,
                                            WL_KEYBOARD_KEY_STATE_RELEASED);
            }
        } else if (!strcmp(cmd, "modifiers")) {
            unsigned long dep = 0, lat = 0, lock = 0, grp = 0;
            sscanf(line, "%*s %lu %lu %lu %lu", &dep, &lat, &lock, &grp);
            zwp_virtual_keyboard_v1_modifiers(vkbd, (uint32_t)dep, (uint32_t)lat,
                                              (uint32_t)lock, (uint32_t)grp);
        } else if (!strcmp(cmd, "keymap")) {
            sscanf(line, "%*s %255s", path);
            int fd = open(path, O_RDONLY);
            if (fd < 0) {
                fprintf(stderr, "cannot open keymap %s\n", path);
                printf("# keymap FAILED\n");
                fflush(stdout);
                return 0;
            }
            off_t sz = lseek(fd, 0, SEEK_END);
            lseek(fd, 0, SEEK_SET);
            zwp_virtual_keyboard_v1_keymap(vkbd, WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1,
                                           fd, (uint32_t)sz);
            close(fd);
        } else {
            fprintf(stderr, "unknown command: %s", cmd);
            printf("# unknown\n");
            fflush(stdout);
            return 0;
        }

        if (pump() < 0) {
            fprintf(stderr, "display connection failed (errno %d)\n", errno);
            return -1;
        }
        printf("# %s", line);
        fflush(stdout);
        return 0;
}

int main(int argc, char **argv) {
    int argi = 1;
    if (argi < argc && !strcmp(argv[argi], "-g")) { use_global = 1; argi++; }
    if (setup() != 0) return 1;

    // One-shot: `-c 'click 410 515'`. Each step runs immediately, with only a
    // short settle between them, and the process exits as soon as the sequence
    // is done.
    if (argi + 1 < argc && !strcmp(argv[argi], "-c")) {
        char *seq = strdup(argv[argi + 1]);
        if (!seq) return 1;
        for (char *tok = strtok(seq, ";"); tok; tok = strtok(NULL, ";")) {
            while (*tok == ' ' || *tok == '\t') tok++;
            if (!*tok) continue;
            int r = run_command(tok);
            if (r < 0) { free(seq); return 1; }
            if (r > 0) break;
            if (pump() < 0) { free(seq); return 1; }
            usleep(120 * 1000);
        }
        free(seq);
        return 0;
    }

    char line[512];
    while (fgets(line, sizeof line, stdin)) {
        int r = run_command(line);
        if (r < 0) return 1;
        if (r > 0) break;
    }

    return 0;
}
