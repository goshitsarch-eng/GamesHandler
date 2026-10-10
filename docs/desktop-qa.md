# Desktop QA record

Baseline checkout: c4fe956d74ee2c1e0fc3736a4be174b41f45e725.
Session dates: 2026-10-09–2026-10-10. Linux x86_64, Debian cloud container, Xvfb/X11 and
Weston/Wayland. Application QA data is isolated under /tmp and
/workspace/.gamehandler-tools/flatpak-qa. No user library was used.
Flutter 3.47.7/Dart 3.13.5 and Rust 1.98.1. Platform compilation checks do not
count as runtime checks.

## Executed

- Original core: 637 passing tests, one ignored. Original full workspace:
  1178 passing tests, one ignored. Python reference: 260 tests, one skipped.
- Original UI opened under Wayland; empty library and settings inspected.
  The original X11 softbuffer path failed on Xvfb's visual, so it was not used
  as evidence of working X11 UI.
- New Rust boundary: nine integration regressions and four unit tests, including Unicode records,
  actual native /bin/true launch, last-played persistence, missing executable,
  corrupt JSON/settings protection, repeated allocation/free and architecture
  selection. Dart calls use the actual Rust library, including concurrent calls.
- Seven Flutter tests: real bridge calls, library fixtures, page navigation,
  editor controls/cancel, 1,500-record virtualization/search and minimum-window
  layout with 200% text scaling and an error banner; portal document mapping.
- Desktop integration executed on Linux: real backend record creation, theme
  update, deletion and visible empty state passed.
- Shared core and test code compile for Windows x64 and macOS ARM64 targets.
- Visible new Flutter library window inspected. Startup ordering was corrected
  after the first runtime attempt exposed a wait-for-frame deadlock.
- Actual release UI: add/save, launch /bin/true, Steam cover download, light/dark
  preferences, search, context actions, keyboard add/cancel, menu quit and window
  geometry persistence. Screenshots are in docs/screenshots.
- Actual delayed game failure: window hidden after launch, restored after a
  two-second script exited 7, with captured stderr visible in the error banner.
- Installed Flatpak tools: Wine 11.0, osslsigncode 2.14, trusted root certificate
  and DXVK DLLs present. Wine initialized an isolated prefix and successfully
  ran `cmd /c echo GAMEHANDLER_WINE_QA`. The real Windows GNU CLI PE was
  then selected through the portal, saved and launched by the UI in a managed
  prefix. It printed its version and exited successfully; last-played persisted.
  The managed prefix's d3d11.dll SHA-256 matched the verified bundled DXVK DLL.
- Installed Flatpak CLI and cold GUI passed after the bzip2 link correction.
  Real portal OpenFile and success response were observed on D-Bus. The selected
  native script was saved and launched, printing its marker; both records and
  the portal grant survived a cold restart, and native launch succeeded again.
  Private D-Bus service paths and the supported FLATPAK executable setting fix
  the cloud image-loader path. The actual portal exposed the host/sandbox mount
  difference; selectedPath now resolves it without extra document grants.
- Core proxy routing regression: 638 tests passed, one ignored. Full workspace:
  1179 passed, one ignored; Clippy passed. The verifier forwards only proxy
  routing for signed CRLs; trust/config/loader overrides remain excluded.
- The official Battle.net bootstrapper downloaded successfully and passed
  Authenticode chain, certificate publisher and revocation verification. The
  packaged Wine loader then rejected the PE32 bootstrapper (ShellExecuteEx,
  exit 1); the UI reported the failure and kept the prefix. No vendor wizard
  completed and no terms were accepted. The original GL32 and Compat.i386
  extensions are inherited and their signed 25.08 runtimes were installed.
  The same verified file failed with executable permission enabled; a disposable
  WINEARCH=wow64 initialization crashed with exit 139. Those diagnostics did not
  change production prefix policy or bypass any verification. Wine32/vendor
  compatibility remains an explicit failing gate despite successful Wine64 CLI QA.
- Final artifacts: Linux x86_64 archive SHA-256
  `c3a4a75bd0ced306ea010203d531ae56b03ec3d3534e3e846db95c5ed28e17ab`;
  installed Flatpak SHA-256
  `2f2db498a4a5aec24cee4ba23fbb5a46eb189250591efcb347390af0e7778ec2`.
- Full macOS backend cross-check was attempted but ring needs the Darwin
  compiler/SDK. Shared-core compilation remains the available macOS evidence.
- Hosted CI run 38002735281 was triggered and inspected. All six jobs were
  stopped before execution by an account billing lock; see ci-verification.json.

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
managed namespace escalation and isolated tooling/private D-Bus. Installed runtime checks caught and corrected a bzip2 SONAME mismatch, an
image-loader executable path and a portal mount mismatch. Runtime, portal and
Wine launch checks passed. QA exposes the existing trusted host CA bundle for
HTTPS through the cloud proxy; it never disables certificate verification.
No GitHub release has been verified. Production certificates are not configured.
The rewrite remains a prerelease until these gates are completed.
