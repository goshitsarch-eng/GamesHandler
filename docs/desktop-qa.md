# Desktop QA record

Baseline checkout: c4fe956d74ee2c1e0fc3736a4be174b41f45e725.
Session date: 2026-10-09. Linux x86_64, Debian cloud container, Xvfb/X11 and
Weston/Wayland. All application data used for checks is isolated under /tmp.
Flutter 3.47.7/Dart 3.13.5 and Rust 1.98.1. Platform compilation checks do not
count as runtime checks.

## Executed

- Original core: 637 passing tests, one ignored. Original full workspace:
  1178 passing tests, one ignored. Python reference: 260 tests, one skipped.
- Original UI opened under Wayland; empty library and settings inspected.
  The original X11 softbuffer path failed on Xvfb's visual, so it was not used
  as evidence of working X11 UI.
- New Rust boundary: six integration regressions, including Unicode records,
  actual native /bin/true launch, last-played persistence, missing executable,
  corrupt JSON/settings protection, repeated allocation/free and architecture
  selection. Dart calls use the actual Rust library, including concurrent calls.
- Flutter widgets: library fixtures, page navigation, editor controls/cancel.
- Desktop integration executed on Linux: real backend record creation, theme
  update, deletion and visible empty state passed.
- Shared core and test code compile for Windows x64 and macOS ARM64 targets.
- Visible new Flutter library window inspected. Startup ordering was corrected
  after the first runtime attempt exposed a wait-for-frame deadlock.

## Required before declaring production parity

Run each on Windows, macOS, native Linux and installed Flatpak. Record OS,
architecture, artifact checksum, observed result and screenshots. Do not mark a
row passing from source inspection.

- Add, edit and cancel; invalid/missing executable; Unicode names/paths; duplicate
  names; remove confirmation; files/prefixes retained; existing JSON fixtures.
- Search/category filters/all three sorts; clear search; grid/list; no overflow
  at 720x480, high DPI, enlarged text and 1000+ records.
- Native launch with spaces/quoted arguments, cwd, environment and auxiliary
  application; immediate failure; keep/hide-window behavior; headless CLI.
- Wine/Proton launch, isolated/custom prefixes, every applicable launch switch,
  winecfg, winetricks, prefix folder, runner detection/removal/architecture.
- Download real runner, progress, cancel, retry; malformed archive/path traversal
  rejection; offline/TLS errors and no security-verification bypass.
- Vendor installer signature verification, wizard lifetime, failure, cancellation,
  automatic executable detection and manual executable selection.
- Steam artwork/custom artwork/executable fallback; missing/corrupt/large images;
  concurrent edit/delete while cover job runs.
- Plugin detection/instructions/copy; browser links; native file dialogs;
  drop executable; keyboard and context menus; macOS native menu bar.
- Light/dark/system themes; defaults preserved after restart; window size;
  focus order, keyboard-only actions, screen readers, high-contrast usability.
- Generated .desktop/.lnk/.app shortcut launches correct record; names containing
  quotes/newlines/percent signs cannot inject a command or escape its directory.
- Install/uninstall/upgrade each artifact with existing user data; bundle ABI,
  library loading, signature/notarization state and permissions.
- Flatpak dialogs/portals, mounted shares, bundled Wine/DXVK/signature verifier,
  sandbox writes, desktop shortcut, native library and cold launch.

## External verification limits

Windows and macOS require native hosted runners. Git proxy pushes and public
Actions inspection are available. Flatpak builds and installation work with
managed namespace escalation and isolated tooling/private D-Bus. An installed
runtime check caught a bzip2 SONAME mismatch; the backend now links its trusted
vendored bzip2 statically. Rebuilt runtime and portal/Wine checks remain pending.
No GitHub release has been verified. Production certificates are not configured.
The rewrite remains a prerelease until these gates are completed.
