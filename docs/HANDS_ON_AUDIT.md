# Hands-on desktop audit — 2026-10-10

Baseline: `708d52a161cd931dfc8e5ca12f1a81023221a608`, 0.9.0-alpha.1.
Linux x86_64, Debian cloud workspace, Flutter 3.47.7 / Dart 3.13.5,
Rust 1.98.1, Xvfb X11. All data uses isolated QA directories.
Linux functional checks passed after nine reproduced defects were fixed. This is
not an unconditional release approval: native Windows/macOS execution, the
32-bit vendor wizard, and an intermittent GTK diagnostic remain external gates.
Source inventory is not execution evidence.

## Control inventory and execution ledger

| Area | Individually inventoried controls / states | Status |
|---|---|---|
| Window | cold launch, close, restore, minimum 720×480, normal, wide, tall, maximize, saved geometry | PASS: cold start, minimum/wide/tall/maximize/restore; geometry restart checked. Launch minimization, restoration and automatic failure restoration passed. |
| Shell | Library, Installers, Runners, Plugins, About, Settings rail; app-bar Add and Refresh | PASS: six destinations, app-bar Add/Refresh and repeated navigation. |
| Menus | Library → Add game, Quit; View → Search, Settings | PASS: every listed entry individually activated. |
| Shortcuts | Ctrl+N, Ctrl+F, Ctrl+comma, Ctrl+Q; Tab/Shift+Tab, Enter, Space, Escape, arrows, Backspace/Delete | PASS: Ctrl+N/F/comma/Q; Tab/Shift+Tab, Space activates the focused navigation item, Escape, text editing, dropdown arrows/Enter. Release keyboard and menu checks repeated; context-menu dismissal regression passes. |
| Library | empty-state Add; toolbar Add; search, category, three sorts, grid/list; card Play/double-click; row Play/click; right-click and overflow menus | PASS: empty/populated/filter/sort/grid/list; native card and row launch. Context actions and double-click exercised in the desktop pass. |
| Game actions | Play, Edit, Find cover art, Choose cover image, Wine configuration, Winetricks, Open prefix folder, Create desktop shortcut, Remove (cancel/confirm) | PASS: all listed actions activated; native custom/Steam artwork, winecfg, prefix folder, winetricks-missing feedback. Final generated shortcut executed successfully through GIO. |
| Editor | Name, executable, four types, category, arguments, working directory, additional app, cover, runner, prefix, environment; five browse buttons; Save/Cancel/Escape | PASS: all fields/types, validation, cancel/save, browse dialogs. Unicode fields preserved. |
| Editor advanced | expand/collapse; MangoHud, GameMode, Prefer SDL, Wine Wayland, HDR, Esync, Fsync, DXVK, VKD3D, NVAPI/DLSS, FSR, BattlEye, EAC, Gamescope, Virtual desktop and size | PASS: all 15 toggles individually switched on/off; virtual desktop size edited; cancel preserves original. |
| Installer catalogue | search, every category, runner selection, each vendor Install; progress, Cancel, Dismiss, Locate executable | PASS error paths: all nine Install controls with unavailable System Wine. Real signed Battle.net download verified; vendor wizard/Locate executable BLOCKED by Wine32 loader. |
| Runner catalogue | eight families, Refresh, each Visit project, release Install, progress/cancel/retry; installed Remove cancel/confirm | PASS: eight live family selectors, project links, Refresh, Install/Cancel/Dismiss; Wine 11.19 install and removal cancel/confirm; prefix retention checked. |
| Plugins | each tool's Install instructions, Copy command, Close, Refresh detection | PASS: all five instruction dialogs, actual clipboard commands, Close and Refresh detection. External privileged package installation is not an app action. |
| About | Flutter/Dart/Rust Visit; each legacy credit Visit; licenses list, individual licenses, back | PASS: three framework and all 27 legacy-credit link handoffs; licenses page. All 158 bundled license package details opened and returned successfully. |
| Settings | system/light/dark, grid/list, three sorts, minimize-on-launch, default runner, each available default toggle | PASS: every theme/layout/sort/default toggle; default runner select/remove exposed BUG-005. Missing-runner display and launch minimization retests passed. |
| Native dialogs | executable, folder, cover/additional-app/prefix picks; cancel, Unicode/spaces, missing/unreadable/wrong file | PASS: executable/folder selection and all five picker cancellation paths; cover file imported. Invalid/missing paths rejected. Unicode filename selected, saved and launched; unreadable and invalid cover imports rejected without replacing the previous cover. |
| Drag/drop | executable onto content; directory/unsupported input; multiple files | PASS: actual Thunar executable drag with spaces, save and launch. Directory Save rejected; multiple drop explains the one-file limit without writing; single drop still opens the editor. |
| Jobs/errors | initial/midflight failure, cancellation, repeated actions, banner Retry, snackbar, stale job completion, navigation during request | PASS: cancellation/retry/dismiss, bounded delayed launch failure, missing tools, no-match cover, navigation while requests complete. Corrupt games/settings bytes preserved; repair and Retry recover. |
| Persistence | records/edits/removal, settings, view/sort, geometry, filters/search consistency; restart; corrupt JSON preservation | PASS: real Rust reload of records, settings and deletion; search/category inconsistencies fixed. Release cold restart retained records/settings; rebuilt Flatpak retained its two records and portal grant. |
| Stress | every page/dialog at minimum and enlarged text; long/Unicode/blank input; 1,500 records, scrolling, quick navigation, repeated dialogs | PASS: five local pages at four sizes and 200% text; 1,500-record virtualization/search. Editor and long-error checks passed at minimum size and 200% text; runner layout is covered by the live network walkthrough. |
| Native Windows/macOS | OS menus, shortcuts, native launch and packaging | BLOCKED: hosts unavailable; hosted CI account billing lock |

## Live defects

### BUG-001 — Hidden active library search (medium)
Reproduction: type an unmatched query, visit Settings, return to Library.
Expected: the query remains visible; Actual: blank field with filtered results.
Root cause: an uncontrolled TextField was recreated while LibraryState retained
its query. Fix: controller initialized from retained state; clear-search and
clear-filter actions, accurate filtered empty-state guidance. File:
lib/ui/library.dart. Retest: PASS in the real desktop integration walkthrough: query remains visible after navigation and Clear filters restores the record.

### BUG-002 — Removed category hides remaining games (medium)
Reproduction: put one of three records in a unique category, select that category,
remove its last record and confirm. Expected: fall back to All and show the two
remaining games. Actual: selector displays All but the old category remains active;
"No games yet" falsely reports an empty library. Screenshot: qa-screenshots/BUG-002-stale-category.png.
Root cause: the dropdown substitutes a display value without updating the query.
Fix: refresh resets a category that no longer exists and re-fetches consistently.
File: lib/state/library_state.dart. Retest: PASS by repeating the filtered removal in the actual desktop; remaining games reappear. Automated regression also checks that the retained category resets to All.

### BUG-003 — Desktop shortcut loses the library context (high)
Area: Library → game actions → Create desktop shortcut.
Reproduction: run the GUI with isolated config/data directories, save a native
script, create its shortcut, and launch the generated file using `gio launch`
without the GUI's environment variables. Actual: “The game no longer exists”.
The shortcut was also written to ~/.local/share despite an absolute XDG_DATA_HOME.
Expected: launch the same record and respect the native desktop's data directory.
Root cause: shortcut commands passed only the game ID; the standalone CLI opened
its default library. Native generation ignored XDG_DATA_HOME.
Fix: absolute `--config-dir`/`--data-dir` CLI options; generated Linux and Windows
commands carry their originating context. Native Linux honors XDG_DATA_HOME;
Flatpak retains its explicitly granted host application directory. Windows command
arguments use CRT-compatible quoting, separate from PowerShell source.
Files: rust/src/main.rs, rust/src/native.rs, rust/src/windows_arguments.rs.
Regression: real CLI reopens a library under a path containing spaces and Unicode;
relative options are rejected; Windows argument quoting round-trips.
Retest: PASS. The final UI-generated shortcut launches the correct fixture using `gio launch` without GAMEHANDLER_CONFIG_HOME/DATA_HOME; the executable bundle path contains spaces, café and a literal %. The child stdout and completion file were observed. GLib rejects a percent-escaped argv[0] before field expansion, so native Linux generation uses `/usr/bin/env --` to pass the encoded launcher as an argument without a shell.

### BUG-004 — Interrupted HTTP reads fail live runner requests (high)
Reproduction: open the actual debug desktop runner catalogue and switch families.
Actual: “io: Interrupted system call (os error 4)” in the rendered page instead
of releases; all eight families were affected. The same library outside Dart
could fetch releases. Expected: retry an interrupted operation within its deadline.
Root cause: blocking HTTP operations propagate EINTR, exposed by VM signals.
Fix: retry interrupted GET attempts before any response reaches callers; retry
interrupted body reads without replaying delivered bytes. Preserve the original
deadline, HTTPS-only redirects, system trust, and callback error propagation.
File: crates/app/src/http.rs (also included by the desktop Rust boundary).
Retest: PASS. All eight live families load; Install → Cancel → Dismiss → Refresh
passed with actual downloads. Unit regression injects interrupted reads around
body chunks and checks exact bytes plus deadline exhaustion.

### BUG-005 — Deleted default runner is displayed as System Wine (medium)
Reproduction: select a downloaded runner in Settings, remove it from Runners,
return to Settings. Actual: System Wine is displayed, while settings.json and new
games retain the deleted runner ID. Expected: accurately show the saved choice
and its unavailable status. Root cause: dropdown substitutes its first choice
when the saved value is absent. Fix: include the retained default as an explicit
“(not installed)” choice; the user can select a replacement without hidden changes.
Files: lib/ui/settings.dart and lib/ui/game_editor.dart. The editor now shows the missing choice instead of a blank runner field. Retest: widget regression PASS; release UI retest PASS, including explicit missing choice and replacement.

### BUG-006 — Hide-on-launch leaves no way back after a successful game (medium)
Reproduction: enable Hide window after launching a game, click a native game row,
and let it exit successfully. Actual: the application remains unmapped, with no
tray control or taskbar entry to restore it. Expected: the user can return to the
library without creating another application process. Root cause: unconditional
window hiding, with restoration only on launch failure.
Fix: minimize instead, label the setting accurately, and explicitly restore before
showing/focusing on a launch failure. The persisted `close_on_launch` key remains
compatible. Files: lib/app/app.dart, lib/main.dart, lib/ui/settings.dart.
Retest: PASS in the release UI: WM_STATE became Iconic after launch, desktop activation restored it, and a delayed exit-7 failure automatically restored WM_STATE to Normal with a readable banner. The actual desktop integration also passes minimize/restore and the long-failure/Retry path at minimum size with 200% text.

### BUG-007 — Invalid imported artwork silently replaces the displayed cover (medium)
Reproduction: choose a text file named `invalid.png` using Choose cover image.
Actual: the record points to the imported invalid data and the card shows a broken
image icon, without an error. Expected: reject unreadable artwork and retain the
current image and record. Root cause: import validated file existence only.
Fix: decode a bounded snapshot before touching existing covers; copy exactly the
validated snapshot to avoid a concurrent source edit changing its contents. Both
editor imports and the game action use the same validator. Limits are 32 MiB
encoded, 128 MiB decoder allocation, and 16384 pixels per side.
Files: rust/src/native.rs, rust/src/api.rs, rust/Cargo.toml and rust/Cargo.lock.
Regression: valid Unicode-path PNG import passes; text disguised as PNG, truncated
PNG and oversized input are rejected through the API; previous managed bytes,
record path and selected source are unchanged; snapshot temporary files are cleaned.
Retest: PASS in Rust and the final release UI. Invalid and unreadable files show errors; the previous good JPEG and record path remain unchanged (SHA-256 checked).

### BUG-008 — Ordinary directories are accepted as executable records (medium)
Reproduction: drag a directory from Thunar, enter a name, choose Linux native,
Save. Actual: an unusable game is saved because only path existence was checked.
Expected: keep the editor open with an actionable validation error. Fix: require
an executable file, with an explicit macOS `.app` directory exception. Files:
rust/src/api.rs, rust/tests/api.rs. Regression: ordinary directories rejected for
all four game kinds without writing a library; macOS app-bundle directories remain
supported. Retest: PASS in Rust and the release UI after an actual Thunar directory drop. The editor remains open with an error and record count is unchanged.

### BUG-009 — Multiple dropped executables silently discard all but the first (low)
Reproduction: select two executable files in Thunar and drag them onto the library.
Actual: the editor opens only the first path without explaining that the second
was ignored. Expected: explain the single-game add workflow. Fix: multiple-file
drops display “Drop one executable at a time.” and make no changes. A single-file
drop continues to open the editor. File: lib/app/app.dart. Retest: PASS with two actual Thunar executable selections; then a single-file drop opens the editor and Cancel leaves the record count unchanged.

The delayed-output native CLI experiment completed successfully: a script wrote
stdout after one second and then wrote its completion marker. This was investigated
as a possible pipe-lifetime problem and is **not** recorded as a defect.

## Evidence

See [screenshots](qa-screenshots/) and [test logs](qa-logs/). The BUG-numbered
before/after images correspond to the reproductions above; BUG-004 is evidenced
by the live runner log. Additional captures show Unicode selection, Wine
configuration, corrupt-state feedback, maximized layout and installed Flatpak
launches. Previous-session results in desktop-qa.md are historical evidence.

## Warnings and external gates investigated

- No native Windows or macOS machine is attached. Windows Rust Clippy cross-target
  passes, but that cannot validate native dialogs, `.lnk` execution, macOS menus,
  signing, or installation. GitHub run 38007652938 could not start any of its six
  jobs because the account is billing-locked. Native OS execution remains blocked.
- Host Wine 11.19 ran the actual Windows x64 CLI PE and winecfg. The verified
  Battle.net PE32 bootstrapper fails in the loader. `/lib/ld-linux.so.2` is absent
  in the host container, while the downloaded distribution includes ELF32 Wine
  components. The installed Flatpak separately failed this vendor path in the
  previous pass despite installed i386 extensions. No vendor wizard completion,
  executable auto-detection, or resulting Locate executable state can be claimed.
- System Wine, winetricks, MangoHud, GameMode, umu and Gamescope are unavailable
  in the native QA PATH; their unavailable states/instructions are exercised.
  Downloaded Wine supplies the tested compatibility path. Rendering HDR, DXVK,
  gamescope and anti-cheat inside actual commercial games requires suitable
  hardware/games; switching their controls is not proof of those runtimes.
- Chromium initially failed its setuid sandbox because this managed filesystem
  cannot provide its expected helper ownership. Using its supported
  `--disable-setuid-sandbox` option with user-namespace sandboxing enabled allowed
  real browser handoff. TLS and application sandbox verification were not disabled.
- External Chromium system D-Bus/UPower/GCM messages, Wine Bluetooth hardware
  messages and Thunar's missing optional thumbnail service were investigated as
  container services/hardware absent from those external processes. XFWM's
  session-manager warning comes from the isolated test desktop. These are not
  Flutter rendering exceptions. An intermittent GTK critical
  `gtk_widget_event: WIDGET_REALIZED_FOR_EVENT` also occurred during scripted
  native-dialog acceptance. Removing forced focus from the helper did not fully
  eliminate it. File selection completed correctly, with no Flutter exception;
  inspection located the native GTK chooser path in file_selector_linux 0.9.4+1.
  Its cause remains unresolved and needs verification on a normal GTK desktop.
- Flatpak under Xvfb reports DRI3 unavailable and uses software rendering. Its
  workspace-extracted session helper also reports an optional p11-kit service
  startup failure. The QA wrapper supplies the verified cloud CA file; native
  and Wine x64 launches pass. These messages are retained, not suppressed.

## Reproducible validation

See BUILDING.md for commands. The audit uses actual Rust-backed records and
native Flutter desktop rendering, plus physical X11 input and GTK/Thunar dialogs;
source inspection and mocked callbacks are not counted as exercised controls.

- Root Rust workspace: **1,180 passed, one ignored**, strict Clippy passed.
- Desktop Rust boundary: **six unit and eleven integration tests passed**, strict
  Clippy passed, including real process launch and CLI directory regression.
- Windows GNU desktop Rust: strict Clippy cross-target passed.
- Flutter unit/widget suite: **seven passed** after all production fixes; analysis,
  formatting and the desktop test driver passed.
- Opt-in live network desktop integration: **PASS (3m04s)** in the final run:
  all 38 browser link handoffs (three frameworks, 27 credits, eight runner
  projects), all eight real download/cancel/dismiss/refresh workflows, four
  window sizes and 200% text. A persistent browser base tab was necessary;
  earlier harness runs exited the browser or used an incorrect desktop entry.
  The final run requires a real browser window for every handoff.
- Main desktop walkthrough: **PASS (7m29s)** after all nine production fixes,
  including all 158 licenses, all editor/default switches, clipboard, minimum
  sizes, 200% text, persistence and corrupt-library recovery.
- Desktop edge walkthrough: **PASS (33s)** for missing runners, minimum-size
  editor, minimize/restore and a long delayed failure with Retry at 200% text.

## Automated regressions added

- `integration_test/audit_test.dart`: real desktop validation, all editor switches,
  filtering, navigation/search retention, removed-category reset, settings,
  clipboard commands, installers' errors, layout, licenses and corrupt-library
  recovery. The integration binding explicitly registers its test text input;
  otherwise synthetic `enterText` does not reach this Linux desktop binding.
- `integration_test/network_audit_test.dart`: opt-in real catalogue/link actions,
  eight runner download cancellations and refresh. It fails when the backend has
  releases but the UI exposes no Install control.
- `rust/tests/api.rs`: standalone CLI with explicit isolated roots containing
  spaces/Unicode, plus rejection of relative directory options.
- `rust/src/windows_arguments.rs`: exact argument round-tripping including empty
  strings, quotes and trailing backslashes.
- `crates/app/src/http.rs`: interrupted body reads preserve bytes and deadlines.
- `test/widget_test.dart`: shortcuts remain available after dismissing a context
  menu; missing saved runner is explicitly represented rather than substituted.

- `integration_test/audit_edge_test.dart`: real window minimize/restore, missing
  runner selection, advanced editor validation and long failure/Retry at 200%.
- `rust/tests/api.rs`: validated artwork preserves existing data on malformed,
  truncated or oversized input; executable-directory rejection preserves the
  library while allowing macOS application bundles.

## Package verification

The final release bundle passed the ABI/CLI verifier, was archived, and was
rebuilt into a Flatpak, installed, and launched. The installed Flatpak retained
its records and portal access; a fresh native portal selection returned the
correct `/run/user/1000/doc/…` path and editor cancellation retained both records.
It successfully launched both a native script
and the Windows x64 CLI under Wine. The installed headless CLI also listed both
QA records using explicit config/data directory options. These artifacts are local build outputs,
not published release assets.

| Artifact | SHA-256 |
|---|---|
| Linux x86_64 tar.gz | `fa8f95919d25ae9fd66126695f596415ee6ce1de02e472d97f5ed3ac7e41837c` |
| Flatpak | `65404b140b05df0a84cc172e68cd425cae757da113a663cd99e4891ad2cd03a3` |
