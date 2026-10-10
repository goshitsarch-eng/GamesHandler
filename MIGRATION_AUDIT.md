# Flutter desktop migration audit

Status: **Flutter/Rust migration implemented as an unreleased alpha; verification is incomplete**. Baseline: existing
`main` checkout, commit `c4fe956d74ee2c1e0fc3736a4be174b41f45e725`.
The existing Rust/libcosmic and Python/QML implementations are retained.

## Evidence and limits

The source, manifests, CI scripts, entry points and existing tests were inspected.
The Rust 1.98.1 toolchain was installed with rustfmt and Clippy. The headless
core suite executed **637 passed, 0 failed, 1 ignored**. The Python reference
suite executed **260 tests, 1 skipped, no failures**; PySide6 is absent, so the
QML smoke test did not run. These are behavior tests, not visual QA.
The original full workspace passed 1178 tests with one ignored, and its full
build passed. The original library/settings screens were inspected on Wayland.
The original X11 path failed on the virtual display visual; that is recorded
rather than treated as a passing runtime check. Source inspection alone does
not establish that a visible control works.

## Original reference implementation

* `crates/app/src/main.rs`: desktop entry point, message dispatch, asynchronous
  orchestration, COSMIC shell and extensive inline tests (11,166 lines).
* `crates/app/src/cli.rs`: `--help`, `--version`, `--list`, `--launch <id>`;
  display-independent entry path.
* `crates/app/src/state.rs`: transient page, form, dialog, worker and notification
  state; six pages, request tokens to discard stale worker replies.
* `crates/app/src/view/`: library, edit form, runners, installers, plugins,
  credits, settings; viewport virtualization and bounded cover decoding.
* `crates/core/src/`: domain model, tolerant JSON, settings, paths, compatibility
  runners, archive security, installer signature verification, covers and credits.
* `gamehandler/`: Python/PySide6/Kirigami reference, not the current shipped UI.
* `data/`: desktop entry, AppStream metadata, icon and Authenticode trust root.
* `build-aux/flatpak/`: Freedesktop 25.08 manifest, Wine BaseApp, pinned cargo
  source generator and vendored source descriptions.
* `scripts/verify.sh`, `scripts/ci.sh`: build, formatting, lint, docs, two Rust
  feature configurations, CLI fixtures, Python parity, packaging and advisory
  gates. `.github/workflows/ci.yml` provides Linux x86_64 and ARM build jobs;
  release workflows package the existing implementation.

Major dependencies: libcosmic/iced git revisions, serde/serde_json, rustix,
tar/zlib/bzip2/lzma, ureq/platform TLS verifier. The renderer is tiny-skia;
the current app needs a display but not a Vulkan GPU. The core uses blocking
operations injected through environment/HTTP/process traits; the app runs
long operations on workers. This seam is useful and should be preserved.

## Persistence and compatibility

`games.json` is an array of 31-field game records, with stable ids, timestamps,
executable/runner/prefix paths, arguments, cover/category/Steam id, kind,
working directory, auxiliary application, custom environment and fifteen
launch switches. `settings.json` holds eighteen preferences. Field order,
Python-style escaping and floating-point round trips are intentional and tested.
Reads tolerate legacy malformed field types; library writes refuse to overwrite
an unreadable/unparsable source. Existing writes use a sibling temporary file
and rename. Windows replace semantics and concurrent writers need validation.

Configuration defaults to `~/.config/gamehandler`; data defaults to
`~/.local/share/gamehandler/{runners,prefixes,covers,downloads}`.
`GAMEHANDLER_CONFIG_HOME`/`GAMEHANDLER_DATA_HOME` override XDG bases.
Runner install metadata, Wine prefixes, cover files and generated shortcuts
are also persistent data and must be inventoried when migration is implemented.
No original user data was rewritten during audit; runtime fixtures are isolated. The Flutter implementation
must retain the JSON schema and Linux locations, use platform directories on
Windows/macOS, and back up data before any unavoidable schema migration.

## Feature inventory

`WORKING` below means exercised by existing headless tests, not manual desktop
verification. `UNKNOWN` requires runtime QA. Migration entries describe implementation; the evidence ledger below separates
executed checks from unrun platform/runtime checks. F = Flutter/Dart;
R = Rust. No row is approved for removal.

| Feature | Current evidence/state | Source | Expected behavior / risks | F responsibility | R responsibility | Platform/migration status |
|---|---|---|---|---|---|---|
| Six-page navigation | UNKNOWN visually | app/state.rs, view/mod.rs | Library, Installers, Runners, Plugins, About, Settings | Navigation/focus | None | All targets; Implemented; verification ledger below |
| Library load/save | WORKING core tests | core/models.rs, json.rs | Preserve ids, JSON and unreadable-file guard | Present errors | Persistence/validation | All targets; Windows writes need tests |
| Add/edit game | UNKNOWN UI | app/view/form.rs, main.rs | Validate name/paths; all 31 fields | Form/dialog state | Validation/store | All targets; Implemented; verification ledger below |
| Remove game confirmation | UNKNOWN UI | main.rs ConfirmDeleteGame | Remove library entry; keep game files and prefix | Confirmation | Remove record | All targets; Implemented; verification ledger below |
| Search/category/sort | WORKING core tests | core/models.rs | Case-folded names, stable sorts, categories | Search/filter widgets | Query operations | All targets; Implemented; verification ledger below |
| Grid/list and virtualization | UNKNOWN visually | view/library.rs | Large libraries remain responsive | Lazy rendering | Batch results | All targets; Implemented; verification ledger below |
| More actions/context menus | UNKNOWN visually | view/widgets.rs, main.rs | Pointer and keyboard routes to same action | Commands/menus | Domain operations | All targets; Implemented; verification ledger below |
| Native game launch | WORKING core fixtures | runners/launch.rs | argv-based launch, cwd/env, error tail and grace watch | Busy/error UI | Process lifecycle | Windows/macOS equivalents required |
| Wine launch | WORKING command tests; real Wine unrun | runners/mod.rs | Prefix isolation, runner detection | Capability presentation | Compatibility adapter | Linux; macOS compatibility not assumed |
| Proton/UMU | WORKING command tests; real launch unrun | runners/mod.rs, launch_opts.rs | Prefer UMU; explain fallback | Capability presentation | Compatibility adapter | Linux-only capability, retain |
| Close/hide on launch | UNKNOWN UI | main.rs, settings.rs | Restore window on launch failure | Window lifecycle | Launch outcome | All targets; Implemented; verification ledger below |
| Last-played tracking | WORKING core tests | models.rs, cli.rs | Only successful process startup updates timestamp | Refresh rows | Mark played | All targets; Implemented; verification ledger below |
| Prefix override/default | WORKING core tests | runners/mod.rs, paths.rs | Per-game isolation | Form | Path policy | Linux Wine; equivalents explicit |
| Winecfg/Winetricks | UNKNOWN runtime | runners/launch.rs tool_command | Run tool against selected prefix | Command/error UI | Process adapter | Capability-based; Implemented; verification ledger below |
| Open prefix folder | UNKNOWN runtime | main.rs OpenPrefixFolder | Platform file manager or portal | Platform plugin | Resolve prefix | All applicable targets |
| Desktop shortcuts/CLI | WORKING core tests | runners/desktop.rs, app/cli.rs | Correct escaping, launch selected id | Platform integration | Launch contract | Windows links/macOS alternatives needed |
| Executable/cover dialogs | UNKNOWN runtime | main.rs, view/form.rs | Cancel safe; portal file access | Native plugin/dialog | Validate selected files | All targets + Flatpak portal |
| Steam cover lookup | WORKING injected tests; live request unrun | core/covers.rs | Search, candidate fallback, provenance | Async results/stale guard | Lookup/cache | All targets; Implemented; verification ledger below |
| Custom cover import | WORKING core tests | core/covers.rs | Copy bounded images; Unicode paths | File chooser | Copy/validation | All targets; Implemented; verification ledger below |
| EXE icons/generated covers | WORKING core tests | exe_icons.rs, covers.rs | Safe PE parsing, generated fallback | Render fallback | Parse/generate | All targets; Implemented; verification ledger below |
| Cover cache bounds | UNKNOWN UI/performance | view/cover_cache.rs | Avoid decoding full library at once | Image cache/lazy widgets | File validation | All targets; Implemented; verification ledger below |
| Eight runner families/releases | WORKING injected tests | runners/families.rs, proton.rs | Official release metadata/asset selection | List/filter/progress | Catalogue/network parsing | Linux runners; Implemented; verification ledger below |
| Runner install/security | WORKING core tests | runners/archive.rs, proton.rs | Bounded extraction, traversal refusal, atomic publication | Progress UI | Archive/network/install | Linux; platform adapter required |
| Runner cancellation/removal | WORKING core tests; UI UNKNOWN | main.rs, proton.rs | Stale jobs do not clear newer guards; confirm deletion | Cancellation/confirmation | Cancellation/install lifecycle | Retain Linux behavior |
| Store installer catalogue | WORKING core tests | installers/mod.rs | Battle.net, Epic, EA, Ubisoft, GOG, Amazon, Rockstar, Steam, Discord | Catalogue/filter | Recipes/capabilities | Native per-platform recipes needed |
| Installer download/signature | WORKING injected tests | installers/download.rs, signature.rs | HTTPS origin checks, magic, Authenticode | Progress/errors | Trust/download engine | Windows/macOS validation unresolved |
| Installer wizard/wait | WORKING injected tests | installers/wizard.rs | Wait for actual prefix processes, locate executable | Async wizard state | Process tracking | Linux adapter; equivalents required |
| Interrupted install/cancel | UNKNOWN runtime | app/easy_install/, main.rs | Keep prefix, manual executable selection, stale replies ignored | Dialog/job state | Job result | Implemented; verification ledger below |
| Plugin detection/install | WORKING injected tests | core/plugins.rs | MangoHud, GameMode, UMU, Winetricks, Gamescope | Capability/status UI | Detection/command policy | Linux-specific capabilities retained |
| Flatpak helper restrictions | WORKING policy tests | plugins.rs | Refuse host package manager installs from sandbox | Explain availability | Capability policy | Flatpak; Implemented; verification ledger below |
| Launch switches | WORKING command tests | runners/launch_opts.rs | MangoHud/GameMode/SDL/Wayland/HDR/Esync/Fsync/DXVK/VKD3D/NVAPI/FSR/anti-cheat/Gamescope/virtual desktop | Grouped controls | Env/argv construction | Explicit per-platform applicability |
| Arguments/custom env/extra app | WORKING parsing tests | runners/shell.rs, launch_opts.rs | Preserve quoted argv; no shell string execution | Edit fields | Parse/validate/launch | Windows argument semantics unresolved |
| SMB/GVFS paths | WORKING injected tests | core/netpaths.rs | Resolve mounted shares; fail clearly if inaccessible | Selected URI handling | Platform path adapter | Windows UNC/macOS volumes needed |
| Theme light/dark/system | UNKNOWN visually | app/theme.rs, view/settings.rs | Follow system changes and persist choice | ThemeData/system observation | Persist preference | All targets; Implemented; verification ledger below |
| View/sort/default runner | WORKING settings tests | core/settings.rs | Persist validated enums | Preferences UI | Settings store | All targets; Implemented; verification ledger below |
| Default launch switches | WORKING settings tests | settings.rs, state.rs | New-game defaults, existing games unchanged | Preferences/forms | Domain default policy | Capability-aware; Implemented; verification ledger below |
| Shortcuts/focus/accessibility | UNKNOWN desktop | shortcuts.rs, view/a11y.rs | Ctrl/Command N/F/comma/Q, focus, semantics | Actions/Shortcuts/semantics | None | All targets; Implemented; verification ledger below |
| Notices/error recovery | UNKNOWN UI | state.rs, main.rs | Actionable errors, retry, no stale callback effects | Notifications/state | Typed errors | All targets; Implemented; verification ledger below |
| About/credits/browser links | UNKNOWN links; data tests WORKING | core/credits.rs, view/credits.rs | Preserve attribution/license and open valid URLs | About/link plugin | Attribution data | Update for Flutter; Implemented; verification ledger below |
| Drag/drop/clipboard/window persistence | Original capability unknown; new implementation present | No audited command entry yet | Establish actual old behavior; desktop improvements required | Plugins/lifecycle | Import validation | All targets; pending audit |
| Releases/Flatpak | Existing configuration; unrun here | .github/workflows/, build-aux/flatpak | Actual bundles/signature checks, multi-OS artifacts | Flutter bundle | Native component | Existing pipeline Linux-only |

## Confirmed architectural issues and open defect candidates

1. The app dispatch/state code is monolithic. A new Flutter command layer should
   own presentation orchestration; retaining this object behind FFI would defeat
   the requested separation.
2. The existing core is GUI-free but **not portable**. Unconditional
   `std::os::unix` imports occur in runner archive, launch, proton and installer
   wizard modules; `renameat2` and POSIX descriptors are Linux assumptions.
   Cross-platform compile claims would be false without platform boundaries.
3. `paths.rs` uses `HOME` with `/` fallback and XDG locations on every target.
   Windows/macOS locations require established platform directory APIs.
4. Store downloads, Linux runner distributions and package-manager commands
   cannot simply be enabled on Windows/macOS. They require capability-specific
   equivalents and honest unavailable states, preserving the Linux workflow.
5. Library unreadable-file protection is present; settings load defaults after
   malformed input, then a later settings save can overwrite the original.
   Investigate/cover this with a regression test before migrating settings.
6. Existing JSON temporary-file naming and replace behavior require concurrency
   and Windows validation. Do not assume POSIX rename proves Windows safety.
7. Runtime/visual defects, dead controls, drag/drop and clipboard behavior remain
   UNKNOWN until the original application can be launched and inspected.

## Implemented boundary

Flutter owns navigation, widgets, menus, theme, focus, desktop shortcuts,
window lifecycle and file/URL/clipboard plugins. Dart owns observable
presentation state and command coordination, not a duplicate game database.
Rust owns compatible data parsing/persistence, domain validation, library
queries, safe archives, executable parsing, download validation and process
policy. Batch APIs should load/query/update libraries, save preferences and
start/manage jobs. Typed failures and request ids cross the bridge.

The selected bridge is a direct versioned C ABI with three symbols. Rust owns
and frees returned buffers; Dart owns input buffers. Coarse JSON requests run
in isolates, and slow jobs return immediately with ids and progress snapshots.
ABI version checks, input bounds, unwind protection and typed errors are tested.
See ARCHITECTURE.md for ownership, concurrency and packaging details.

## Migration evidence ledger

- `lib/` contains the six-page Flutter shell, forms, library query presentation,
  theme/preferences, keyboard/native menus, context menus, drag/drop, clipboard
  command copying, native dialogs/URLs and window-size persistence.
- `rust/` reuses shared domain behavior, adds absolute-context request validation,
  corrupt-settings protection, job registry, OS file locking, platform launch,
  portable cover jobs, native shortcuts and a display-independent CLI.
- Runner architecture selection was an actual defect: a first ARM asset could
  be chosen on x86_64. Selection now filters architecture and has a regression.
- New backend: nine integration regressions and four unit tests pass. Flutter: seven tests pass against
  the real library; analysis and Rust Clippy pass without warning suppression.
  Linux desktop integration passes (Unicode CRUD and theme through the real ABI).
- Portable core and tests compile for Windows x64 and macOS ARM64. These are
  compile checks, not claims that the desktop UI or package runs there.
- The new visible Linux library was inspected. Startup ordering was fixed after
  actual runtime testing exposed a wait-for-frame deadlock. A later concurrent
  debug/release build produced a mismatched engine; builds must run sequentially.
- Installed Flatpak startup caught a bzip2 SONAME mismatch; static linking
  against the existing vendored bzip2 fixed it without a loader alias.
- Verifier isolation removed proxy routing needed for signed CRL retrieval.
  Only proxy routing variables are now forwarded; trust-store overrides,
  OpenSSL configuration, PATH and loader hooks remain excluded. An actual child
  process test proves that boundary. Signatures, revocation and publisher checks
  remain mandatory. Current core tests: 638 passed, one ignored; full workspace:
  1179 passed, one ignored.
- The reusable cloud install script was executed successfully and its full
  contents/start instructions saved as an environment draft. Saving does not
  publish/apply that configuration.

## Intentional platform behavior and unresolved gaps

Linux retains Wine/Proton, isolated prefixes, runner downloads, verified vendor
installers and optional helpers. Windows/macOS use native application launch and
official vendor installer links, not a pretend Wine wizard. macOS Windows-game
launch is explicitly unsupported without a separate compatibility integration.
Host plugin installation presents/copies a validated command; it never silently
executes a package manager. Native shortcuts are .lnk on Windows and .app on
macOS; those implementations still need target-host tests.

Windows/macOS require hosted native runners. Git proxy pushes and public Actions
inspection are available. Flatpak namespaces work with the managed sandbox
escalation; verified SDK/BaseApp installation, a complete build and user
installation succeeded. The installed CLI exposed a bzip2 SONAME mismatch,
corrected by statically linking the existing trusted vendored dependency.
The rebuilt installed CLI and GUI run. Real portal selection returned a host
document path different from the sandbox mount; the UI now asks the document
portal for its mount point and maps only documents inside that mount to the
sandbox runtime, checking accessibility before use. Save and native launch
succeeded through that path, including after a cold restart. A Windows PE CLI
ran through bundled Wine in a managed prefix; the installed DXVK DLL hash
matched the verified bundled DLL.
The official Battle.net bootstrapper passed signature, publisher and revocation
checks but failed at Wine32 loading with exit 1; its prefix was retained and
the UI surfaced the failure. Inherited GL32/Compat.i386 dependencies were
installed. Vendor wizard completion, large-library accessibility/manual testing
and production signing remain open. The new Flatpak manifest currently targets x86_64. Legacy
clients do not share the new cross-process lock, so concurrent edits with them
are unsupported. Delayed launch failures are observed by Rust and request window restoration;
the observer is integration-tested. Actual window mapping was observed hidden
for 1.2 seconds and visible again after the delayed fixture failed; stderr was
displayed in the restored window. These are completion gates, not passing
features or grounds to declare the requested rewrite complete.

## Required completion evidence

Complete the per-target checklist in docs/desktop-qa.md, execute and inspect
.github/workflows/desktop.yml and desktop-release.yml, verify installed Flatpak
portals and native loading, and inspect the published artifact set before a
production-complete claim. PLATFORM_SUPPORT.md records target-specific evidence.
Hosted workflow [38002735281](https://github.com/goshitsarch-eng/GamesHandler/actions/runs/38002735281)
was pushed, triggered and inspected through the Actions API. All six jobs were
prevented from starting: "The job was not started because your account is locked
due to a billing issue." See docs/ci-verification.json. No target-host job passed
and no GitHub release has been published.
