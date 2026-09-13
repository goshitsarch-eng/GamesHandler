# GameHandler — advertised vs implemented

**Scope.** This document is the advertised-vs-implemented contract for the Rust
workspace (`crates/core` = `gamehandler-core`, `crates/app` = `gamehandler`).
It answers one question per row: *does something the project claims to a user
correspond to code that does it?* It does **not** judge whether the code is
correct — that is `BUGS.md` — and it does not cover security posture, which is
`SECURITY.md`.

**Method.** The advertised side is read from four places, in this order of
authority:

1. `docs/migration/PLAN.md` §4 (lines 129–310) — `P-01`…`P-78`, `B-01`…`B-08`,
   `N-01`/`N-02`. This is the project's own acceptance standard.
2. `data/com.goshapps.GameHandler.metainfo.xml` — the per-release user-facing
   notes.
3. `README.md` — the feature list a new user reads.
4. The UI itself (`crates/app/src/view/*.rs`) — every control, tooltip, tab,
   label and setting row.

The implemented side is read from `crates/` and, where a claim is about
behaviour rather than structure, **executed**. Claims I could not verify by
reading or running are marked `unverified` and say so.

**Status vocabulary.** `Complete` — the advertised behaviour is implemented and
I read the code that does it. `Partial` — implemented with a named gap.
`Missing` — no implementation found. `Stub` — a control or value exists that
cannot do what it appears to do.

---

## 0. Premise corrections

Two premises in the audit brief do not match the tree at `d56782d`, and both
change how the rest of this document should be read.

| # | Premise as given | What the tree holds | Evidence |
|---|---|---|---|
| 0.1 | "the Python 3 + PySide6 + QML reference (**now deleted from the tree**)" | The Python reference is **present and runnable**. `gamehandler/` holds `bridge.py` (1,072 lines), `main.py`, `models.py`, `settings.py`, `runners.py`, `installers.py`, `covers.py`, `exe_icons.py`, `netpaths.py`, `plugins.py`, `credits.py`, `theme.py`, `config.py`, and thirteen `.qml` pages; `tests/` holds the Python suite. | `gamehandler/main.py:1`, `gamehandler/bridge.py:1`, `gamehandler/qml/Main.qml:1`, `tests/test_security.py:1` |
| 0.2 | (implied) the port is a live migration in progress, so unlanded pages are expected | Every page is wired. `PENDING_PAGES` is the empty slice at `crates/app/src/main.rs:3576`, and `view_body` calls a real body for all nine pages (`crates/app/src/main.rs:1355-1441`). | `crates/app/src/main.rs:3576`, `crates/app/src/main.rs:1401` |

0.1 is a **material** correction and it is a favourable one: the reference is not
a memory to be reconstructed, it is a second oracle that can be run beside the
frozen fixtures in `docs/migration/oracle/`. Every parity claim below was
re-derived against it. Where the brief said "if you cannot verify, mark it
unverified", the Python tree made many such items verifiable instead.

0.2 means the *deferral* caveats scattered through the tree are now stale. They
are catalogued in §6 rather than treated as live.

---

## 1. `README.md`'s feature list

`README.md:31-59` is the "Features" section. Read against `crates/`:

| Feature | Where advertised | Where implemented | Status |
|---|---|---|---|
| Library grid / list with cover tiles | `README.md:33-35` | `crates/app/src/view/library.rs:296-325` (view toggle), `crates/app/src/view/widgets.rs:352-422` (card), `:463+` (row) | Complete |
| Search across name and category | `README.md:36` | `crates/core/src/models.rs:680-694` (`name` **or** `display_category`, both lowercased) | Complete |
| Category filter + sorting | `README.md:36` | `crates/app/src/view/library.rs:320-321` (`SORT_OPTIONS`), `crates/core/src/models.rs:626-643` (`all()`, three orders with Python's tie-breaks) | Complete |
| Generated cover art (initials plate, gradients) | `README.md:38-40` | `crates/core/src/covers.rs:224` (`initials`), `:306` (`GENRE_MAP`), `:520` (`accent_index`) | Complete |
| Steam artwork lookup + exe-icon fallback | `README.md:41-43` | `crates/core/src/covers.rs:700-1018` (search → score → CDN), `:590` (`save_exe_icon`), `crates/core/src/exe_icons.rs` | Complete |
| Eight runner families + System Wine | `README.md:44-46` | `crates/core/src/runners/families.rs` (catalogue), `crates/app/src/view/runners.rs:172-207` (`release_rows`) | Complete |
| Fifteen launch helpers (MangoHud, GameMode, …) | `README.md:47-49` | `crates/app/src/state.rs:266` (`TOGGLE_NAMES`, 15), `crates/app/src/view/form.rs:286` (`LAUNCH_TOGGLES`, 8) + `:341` (`COMPAT_TOGGLES`, 7) | Complete |
| Nine easy installers | `README.md:50-52` | `crates/core/src/installers.rs:247` (`INSTALLERS`, 9) | Complete |
| Plugin/helper detection | `README.md:53-55` | `crates/core/src/plugins.rs:90` (`PLUGINS`, 5), `crates/app/src/state.rs:920` (`refresh_plugins`) | Complete |
| Credits page | `README.md:56` | `crates/core/src/credits.rs:94` (`CREDIT_SECTIONS`, 5), `crates/app/src/view/credits.rs:326-354` | Complete |
| Desktop shortcuts | `README.md:57` | `crates/core/src/runners/desktop.rs:192-257`, `crates/app/src/main.rs:2867` (`shortcut_command`) | Complete |
| GVFS network-share games | `README.md:58` | `crates/core/src/netpaths.rs:71-130`, `crates/app/src/state.rs:448-450` | Complete |
| "the Python application … still runs exactly as described" | `README.md:15-16` | `gamehandler/` is intact | Complete — and see §0.1 |
| "the library pages … land task by task — check `PLAN.md` §6 before relying on a feature here" | `README.md:110-111` | Nothing is deferred: `crates/app/src/main.rs:3576`, `crates/app/src/view/installers.rs:137` | **Stale** — the caveat tells a reader to distrust a finished app |

The `README.md:63-64` comparison table bills the Rust port as `0.8.0` and the
Python reference as `0.7.2`; `Cargo.toml:18` holds `version = "0.8.0"` and
`--version` prints `GameHandler 0.8.0` (executed). Consistent.

---

## 2. `metainfo.xml`'s per-release claims

Each release note is a promise to a user who upgrades. Read against `crates/`:

| Feature | Where advertised | Where implemented | Status |
|---|---|---|---|
| 0.6.1 — "Easy installers now wait for the vendor's own wizard" | `data/com.goshapps.GameHandler.metainfo.xml` (0.6.1) | `crates/core/src/installers.rs` (`wait_for_installer`, `wait_for_prefix_idle`), `crates/app/src/main.rs:3366` (`easy_install_wizard_finished`) | Complete |
| 0.6.1 — "a Proton prefix keeps its files one directory deeper, which used to make every Proton install look like a missing executable" | same | `crates/app/src/main.rs:2788-2802` (`prefix_drive_c`), `crates/core/src/installers.rs` (`find_prefix_exe`) | Complete |
| 0.6.1 — "artwork lookup no longer attaches the wrong cover" | same | `crates/core/src/covers.rs:430-487` (`pick_best_match` + `MINIMUM_MATCH_SCORE`) | Complete |
| 0.6.1 — "a game that exits the moment it starts now reports why" | same | `crates/core/src/runners/launch.rs:102-200` (non-blocking drain), `:252-287` (`failure`), `crates/core/src/runners/mod.rs:938-947` (`failure_message`) | Complete — this is `B-07`, and it is the port's one *deliberate* divergence from the reference (comment at `crates/core/src/runners/launch.rs:185-193`) |
| 0.5.0 / 0.4.0 / 0.3.0 notes | metainfo | in `crates/` throughout | Complete — not itemised here; each is a subset of the `PLAN.md` §4 rows in §3 |

---

## 3. `PLAN.md` §4 — the acceptance standard

### 3A. Library (`P-01`…`P-17`)

| Feature | Where advertised | Where implemented | Status |
|---|---|---|---|
| `P-01` Grid view with cover tiles | `docs/migration/PLAN.md:135` | `crates/app/src/view/widgets.rs:352-422` | Complete |
| `P-02` List view: thumbnail, subtitle, last-played | `docs/migration/PLAN.md:136` | `crates/app/src/view/widgets.rs:463+`, `crates/core/src/models.rs:466` (`format_last_played`) | Complete |
| `P-03` Grid/list toggle persisted | `docs/migration/PLAN.md:137` | `crates/app/src/main.rs:239` (`SetViewMode` → `save_settings_or_toast`), `crates/core/src/settings.rs:60` | Complete |
| `P-04` Search matches name **or** category | `docs/migration/PLAN.md:138` | `crates/core/src/models.rs:689-692` | Complete |
| `P-05` Category filter | `docs/migration/PLAN.md:139` | `crates/app/src/main.rs:2304` (`SetCategoryFilter`), `crates/core/src/models.rs:683-685` | Complete |
| `P-06` Sort: name / recently played / added | `docs/migration/PLAN.md:140` | `crates/core/src/models.rs:626-643` | Complete |
| `P-07` Blank category → "Uncategorized", sorted last | `docs/migration/PLAN.md:141` | `crates/core/src/models.rs:698-708` | Complete |
| `P-08` Empty-library placeholder, 3 onboarding actions | `docs/migration/PLAN.md:142` | `crates/app/src/view/library.rs:336-350` (`ADD_FIRST_GAME`, `EASY_INSTALL`, `DOWNLOAD_A_RUNNER`) | Complete |
| `P-09` No-results placeholder + clear filters | `docs/migration/PLAN.md:143` | `crates/app/src/view/library.rs:352-356`, `:111` (`CLEAR_FILTERS`) | Complete |
| `P-10` Double-click plays | `docs/migration/PLAN.md:144` | `crates/app/src/view/widgets.rs:422` (`.on_double_click(on_play)`), `:463+` for the row | Complete |
| `P-11` Right-click context menu (**9 items**) | `docs/migration/PLAN.md:145` | `crates/app/src/view/library.rs:534-590` — **eight** items | Complete — the *count in the standard is wrong*, not the code. The reference `gamehandler/qml/LibraryPage.qml:298-343` also has eight, and `crates/app/src/view/library.rs:543` says "eight items and two dividers" and cites it entry for entry |
| `P-12` Play / Edit / Find cover art | `docs/migration/PLAN.md:146` | `crates/app/src/view/library.rs:516-529` (`GameMenuAction::message`) | Complete |
| `P-13` Winecfg / Winetricks / Open prefix, disabled for Linux | `docs/migration/PLAN.md:147` | `crates/app/src/view/library.rs:549-575` (`prefix = !game.is_linux()`), `:680-695` (`row_enabled`) | Complete |
| `P-14` Create desktop shortcut | `docs/migration/PLAN.md:148` | `crates/core/src/runners/desktop.rs:192-257` | Complete |
| `P-15` Remove with confirm; prefix left on disk | `docs/migration/PLAN.md:149` | `crates/app/src/main.rs:1141` (`remove_game_dialog`), `:1685-1693` (`library.remove` only; the prefix is never touched) | Complete |
| `P-16` "Last played" labels incl. future-timestamp clamp | `docs/migration/PLAN.md:150` | `crates/core/src/models.rs:466-513` (`.max(0.0)`, NaN-aware, "Never played" for `0.0`) | Complete |
| `P-17` Runner label per row | `docs/migration/PLAN.md:151` | `crates/app/src/view/widgets.rs` (meta/runners), `crates/core/src/runners/mod.rs:611-621` (`family_label`) | Complete |

### 3B. Add/Edit form (`P-18`…`P-31`)

| Feature | Where advertised | Where implemented | Status |
|---|---|---|---|
| `P-18` Add (Ctrl+N) / Edit; Save gated on non-blank name | `docs/migration/PLAN.md:157` | `crates/app/src/view/form.rs:503` (`can_save`), `:514` (`save_message`), `:833` (`.on_press_maybe`), `crates/app/src/shortcuts.rs:82+` (Ctrl+N) | Complete |
| `P-19` Windows vs Linux-native; Linux disables runner/prefix/Wine toggles | `docs/migration/PLAN.md:158` | `crates/app/src/view/form.rs:418-436` (`kind_index`/`kind_is_linux`/`kind_selection`), `:678-695` (`row_enabled`), `crates/app/src/state.rs:421` (`SetFormLinux`) | Complete |
| `P-20` Executable browse (`*.exe`), auto-fill name from basename | `docs/migration/PLAN.md:159` | `crates/app/src/view/form.rs:3220` (`exe_file_filters` — see `crates/app/src/main.rs:1720-1745` for the basename fill); test `crates/app/src/main.rs:7635` | Complete |
| `P-21` `smb://` / `file://` resolved on save **and** browse | `docs/migration/PLAN.md:160` | `crates/app/src/state.rs:448-450` (save), `crates/app/src/main.rs:3337`/`:3350` (browse) | Complete |
| `P-22` Launch args, working directory | `docs/migration/PLAN.md:161` | `crates/core/src/runners/mod.rs:391-397` (`shlex.split`), `crates/core/src/runners/launch.rs:386` | Complete |
| `P-23` Editable category combo (preset + custom) | `docs/migration/PLAN.md:162` | `crates/app/src/view/form.rs:399-416` (`form_categories`), `:475` (`category_index`) | Complete |
| `P-24` Cover preview + Find cover + custom import (png/jpg/jpeg/webp) | `docs/migration/PLAN.md:163` | `crates/app/src/main.rs:3311` (`image_file_filters`), `:664-737` (`FetchCoverForForm`/`FormCoverFetchFinished`) | Complete |
| `P-25` Steam genre auto-categorises when Uncategorized | `docs/migration/PLAN.md:164` | `crates/core/src/covers.rs:306` (`GENRE_MAP`), `crates/app/src/main.rs:361-402` (`SaveGameForm` + `FetchCover` chain) | Complete |
| `P-26` Runner picker + optional prefix | `docs/migration/PLAN.md:165` | `crates/app/src/view/form.rs:445-470` (`runner_index`/`runner_selection`), `crates/core/src/runners/mod.rs:365-371` (`game_prefix`) | Complete |
| `P-27` All 15 per-game toggles | `docs/migration/PLAN.md:166` | `crates/app/src/state.rs:266` (15 names), `crates/app/src/view/form.rs:286`+`:341` (8+7 rows) | Complete |
| `P-28` Virtual-desktop size, validated, default 1920x1080 | `docs/migration/PLAN.md:167` | `crates/app/src/state.rs:476-484`, `:513` (`DEFAULT_DESKTOP_SIZE`), `crates/core/src/runners/launch_opts.rs:532-560` (`is_desktop_size`) | Complete |
| `P-29` Additional app launched alongside | `docs/migration/PLAN.md:168` | `crates/core/src/runners/launch.rs:366-381` | **Partial** — the helper is spawned, but a spawn failure is discarded, so a helper that never started is indistinguishable from one that did. See `BUGS.md` `BUG-05` |
| `P-30` Custom `KEY=value` env, overrides toggles, quoting/`;` | `docs/migration/PLAN.md:169` | `crates/core/src/runners/launch_opts.rs:211-244` (`parse_env_block`), `:762` (block re-applied last) | Complete — verified against `python3 shlex` semantics; test at `:989` pins the `A=1 B` case |
| `P-31` Auto cover fetch on save when empty | `docs/migration/PLAN.md:170` | `crates/app/src/main.rs:359-368` (`FetchCover` batched into `SaveGameForm`) | Complete |

### 3C. Runners (`P-32`…`P-39`)

| Feature | Where advertised | Where implemented | Status |
|---|---|---|---|
| `P-32` Installed list: System Wine + downloaded builds | `docs/migration/PLAN.md:176` | `crates/core/src/runners/mod.rs:1045-1068` (`installed_protons`), `crates/app/src/view/runners.rs:172` | Complete |
| `P-33` 8 families + System Wine guide row | `docs/migration/PLAN.md:177` | `crates/core/src/runners/families.rs` (catalogue + guide rows), `crates/app/src/view/runners.rs` | Complete |
| `P-34` Per-family release list (≤12) | `docs/migration/PLAN.md:178` | `crates/app/src/view/runners.rs:755` (`RELEASES_LIMIT = 12`, from `gamehandler/bridge.py:702`) | Complete |
| `P-35` Download with progress, busy guard, completion toast | `docs/migration/PLAN.md:179` | `crates/app/src/view/runners.rs:974-1025` (`runner_busy`, `progress`, `RunnerInstallFinished`) | **Partial** — the download, its progress and its guards are all real, but what it downloads cannot be installed: the validation that runs next rejects every real Proton archive (`BUG-35`), so the progress bar ends in a refusal |
| `P-36` Release asset filtering per family (CachyOS excludes `v3`/`znver4`) | `docs/migration/PLAN.md:180` | `crates/core/src/runners/families.rs:127` (`exclude: &["v3", "znver4", "native"]`), `:296` (`asset_matches_tokens`) | Complete |
| `P-37` Removal with confirm; games fall back to System Wine | `docs/migration/PLAN.md:181` | `crates/app/src/view/runners.rs:1052-1068` (`ConfirmRemoveRunner`/`RemoveRunnerConfirmed`), `:1120-1127` (`remove_runner`) | **Partial** — the fallback works, but a *symlinked* build is listed as installed and its removal reports success without deleting. See `BUGS.md` `BUG-03` |
| `P-38` Secure extraction: traversal/symlink confinement, caps, no-replace rename | `docs/migration/PLAN.md:182` | `crates/core/src/runners/archive.rs` | **Partial** — the confinement checks are sound and the extraction filter is correct, but the post-extraction symlink walk mis-computes each link's parent as `""`, so it refuses every `..`-relative link and **no real Proton archive can be installed at all** (`BUG-35`); the containment root also degrades to a lexical comparison when `canonicalize` fails (`BUG-26`) |
| `P-39` Tags sanitised to collision-resistant install ids | `docs/migration/PLAN.md:183` | `crates/core/src/runners/families.rs:557-590` (`install_id`, `~i{digest[..12]}` fallback), `crates/core/src/runners/archive.rs:232-242` (`safe_install_id`) | Complete |

### 3D. Launch semantics (`P-40`…`P-50`)

| Feature | Where advertised | Where implemented | Status |
|---|---|---|---|
| `P-40` Proton via `umu-run`, plain-wine fallback | `docs/migration/PLAN.md:189` | `crates/core/src/runners/mod.rs:735-745` (`build_command`), `crates/core/src/runners/launch.rs:336` | Complete |
| `P-41` Proton-only gates raise on plain Wine | `docs/migration/PLAN.md:190` | `crates/core/src/runners/launch.rs:335-345` (`NvapiNeedsProton`, `FsrNeedsProton`, `WaylandNeedsProton`) | Complete |
| `P-42` Esync/Fsync/DXVK-off/VKD3D-off env + dll overrides | `docs/migration/PLAN.md:191` | `crates/core/src/runners/launch_opts.rs:636-764` | Complete — with a cosmetic `;`-trim divergence (`BUG-22`) |
| `P-43` MangoHud/GameMode/Gamescope wrapping; gamescope-missing error | `docs/migration/PLAN.md:192` | `crates/core/src/runners/launch_opts.rs:738-749` | Complete — note that `gamemode`/anticheat missing binaries are silently ignored where gamescope raises — and that is Python's behaviour too (`gamehandler/runners.py:1240-1253`, `:1220-1228`), so this row is Complete and the asymmetry is a note rather than a defect |
| `P-44` Anti-cheat runtimes auto-located | `docs/migration/PLAN.md:193` | `crates/core/src/runners/env.rs:216-224` (seven roots incl. Flatpak Steam) | Complete |
| `P-45` Bundled DXVK installed into raw-Wine prefixes once per version | `docs/migration/PLAN.md:194` | `crates/core/src/runners/launch_opts.rs:420-436`, `crates/core/src/runners/launch.rs:355` | **Partial** — an empty `GAMEHANDLER_DXVK_ROOT` silently skips the install where the reference aborts. See `BUGS.md` `BUG-06` |
| `P-46` Immediate-failure detection: grace, stderr tail, toast + window restore | `docs/migration/PLAN.md:195` | `crates/core/src/runners/launch.rs:102-200`, `:252-287`; `crates/app/src/main.rs:2689` (`launch_grace`), `:2718` (`launch_and_watch`) | Complete |
| `P-47` `mark_played` + "Launching…" toast | `docs/migration/PLAN.md:196` | `crates/app/src/main.rs:2099` (`mark_played`), `:2100+` (toast) | Complete |
| `P-48` Winecfg/Winetricks use the game's own WINE/WINESERVER | `docs/migration/PLAN.md:197` | `crates/app/src/main.rs:2748` (`start_prefix_tool`), `crates/core/src/runners/launch.rs:468-480` (`tool_command`) | Complete |
| `P-49` Open prefix folder (Proton `pfx` aware) | `docs/migration/PLAN.md:198` | `crates/app/src/main.rs:2788-2810` (`prefix_drive_c`, `open_prefix_folder`) | **Partial** — the `pfx` awareness is right, but the spawn result is discarded so a failure is reported as success. See `BUGS.md` `BUG-04` |
| `P-50` Linux-native launch (no Wine env) | `docs/migration/PLAN.md:199` | `crates/core/src/runners/mod.rs` (`is_linux` short-circuit), `crates/core/src/runners/launch.rs` | Complete |

### 3E. Easy installers (`P-51`…`P-59`)

| Feature | Where advertised | Where implemented | Status |
|---|---|---|---|
| `P-51` Catalog of 9 store launchers | `docs/migration/PLAN.md:205` | `crates/core/src/installers.rs:247` (`INSTALLERS: [Installer; 9]`) | Complete |
| `P-52` Search + Launchers/Apps filter | `docs/migration/PLAN.md:206` | `crates/core/src/installers.rs:465` (`search`), `crates/app/src/view/installers.rs:137-141` (`installer_categories`, now chaining `INSTALLER_CATEGORIES`) | Complete — this was `PLAN.md:388`'s `T-36` defect ("the dropdown looks implemented and cannot filter"); it is fixed |
| `P-53` Per-install runner choice, isolated prefix | `docs/migration/PLAN.md:207` | `crates/app/src/main.rs:2381-2390` (`SetInstallRunner`), `crates/core/src/installers.rs:1057-1064` (`prepare_prefix`) | Complete |
| `P-54` Origin allowlist + magic bytes + Authenticode before execution | `docs/migration/PLAN.md:208` | `crates/core/src/installers.rs:1347-1371` (origin), `:1486-1524` (Authenticode) | Complete — the *strength* of the Authenticode decision is a security finding, not a completeness one; see `SECURITY.md` `SEC-03` |
| `P-55` msi via `msiexec`, exe directly | `docs/migration/PLAN.md:209` | `crates/core/src/installers.rs` (`installer_argv`) | Complete |
| `P-56` Wizard wait: poll, wineserver slicing, 6h ceiling, case fallbacks | `docs/migration/PLAN.md:210` | `crates/core/src/installers.rs` (`wait_for_installer`, `wait_for_prefix_idle`, `resolve_case_insensitive`) | Complete |
| `P-57` Fallback "Locate exe" dialog; cancel keeps prefix | `docs/migration/PLAN.md:211` | `crates/app/src/main.rs:3195` (`locate_exe_task`), `:3366` (`easy_install_wizard_finished`), `:3443` (`cancel_easy_install`) | Complete |
| `P-58` Finished install → library entry with exe-icon cover + Play toast | `docs/migration/PLAN.md:212` | `crates/app/src/main.rs:3362` (`installed_play_message`), `:3414` (`complete_easy_install`) | Complete |
| `P-59` Concurrent-install guard | `docs/migration/PLAN.md:213` | `crates/app/src/main.rs:777-784` (`running_install` guard on `StartEasyInstall`), `crates/app/src/state.rs:952` (`refresh_installers`) | Complete |

### 3F. Covers (`P-60`…`P-63`)

| Feature | Where advertised | Where implemented | Status |
|---|---|---|---|
| `P-60` Steam search → scored match → portrait cover, CDN fallbacks | `docs/migration/PLAN.md:219` | `crates/core/src/covers.rs:367` (`parse_store_search`), `:430-487` (`pick_best_match`), `:489` (`cover_urls_for_app`), `:51-67` (`CDN_ROOTS`) | Complete |
| `P-61` Offline exe-icon fallback; launchers get vendor icon | `docs/migration/PLAN.md:220` | `crates/core/src/covers.rs:590` (`save_exe_icon`), `crates/core/src/exe_icons.rs` | **Partial** — a *write* failure in the icon path is discarded and the user is shown the Steam error instead. See `BUGS.md` `BUG-13` |
| `P-62` Initials plate, 8 stable gradients, letterboxed icons | `docs/migration/PLAN.md:221` | `crates/core/src/covers.rs:224` (`initials`), `:520`/`:1059` (`accent_index`/`accent_index_in`) | Complete |
| `P-63` Download size caps, empty-download rejection, atomic writes | `docs/migration/PLAN.md:222` | `crates/core/src/covers.rs:71` (`MAX_RESPONSE_BYTES`), `:73` (`MIN_COVER_BYTES`), `:590-620` (tmp + rename) | Complete |

### 3G. Shell, settings, plugins, credits, CLI (`P-64`…`P-78`)

| Feature | Where advertised | Where implemented | Status |
|---|---|---|---|
| `P-64` Plugins: 5 helpers, 3 states; Flatpak never offers host install | `docs/migration/PLAN.md:228` | `crates/core/src/plugins.rs:90` (5), `:244` (`in_flatpak`), `:304` (the refusal string); `crates/app/src/view/plugins.rs:90-177` | Complete |
| `P-65` About & Credits: 5 sections, licenses, links, Gosh identity | `docs/migration/PLAN.md:229` | `crates/core/src/credits.rs:94` (`CREDIT_SECTIONS`, 5 entries at `:95,142,186,232,276`) | Complete |
| `P-66` Settings: theme, grid/list, default runner, 13 toggles, close-on-launch | `docs/migration/PLAN.md:230` | `crates/app/src/view/settings.rs:68` (`DEFAULT_TOGGLES`, 13), `:503` (`close_on_launch`) | Complete — the 13-vs-15 difference from `P-27` is the reference's own (`crates/app/src/view/settings.rs:63-67`) |
| `P-67` Theming: **light/dark/system** | `docs/migration/PLAN.md:231` | `crates/app/src/theme.rs:114` (`theme_for`), `:163` (`apply`); called at startup from `crates/app/src/main.rs:3717` with the *stored* scheme and on change from the `SetColorScheme` arm | Complete — see §6.3 for the module doc's own (accurate) statement that the `init` call site is not covered by a test |
| `P-68` Shortcuts Ctrl+N / Ctrl+F / Ctrl+, / Ctrl+Q | `docs/migration/PLAN.md:232` | `crates/app/src/shortcuts.rs:82+` (exact-modifier guard, four arms) | **Partial** — the reference's `Qt.ApplicationShortcut` semantics are not reproduced: a focused `text_input` swallows the keys. Recorded in `crates/app/src/shortcuts.rs`'s module doc and in `docs/migration/REPORT.md:94-97` |
| `P-69` Toasts for every mutation; install toast has a Play action | `docs/migration/PLAN.md:233` | `crates/app/src/main.rs:2603` (`toast_task`), `:3362` (`installed_play_message`) | Complete |
| `P-70` `--list` / `--launch ID` / `--version` work headless | `docs/migration/PLAN.md:234` | `crates/app/src/main.rs:99-146` (dispatch before anything GUI-shaped, `D-12`) | Complete — executed: `--version` → `GameHandler 0.8.0` rc 0; `--list` on a 1-game library → `alpha-1\tAlpha` rc 0; `--launch alpha-1` → the runner-unavailable message rc 1 |
| `P-71` Desktop shortcuts `gamehandler --launch <id>`, escaping `%`/newline | `docs/migration/PLAN.md:235` | `crates/core/src/runners/desktop.rs:109` (`escape_desktop_value`), `:129` (`desktop_exec`) | Complete |
| `P-72` Network-share games via GVFS; unmounted → instructional error | `docs/migration/PLAN.md:236` | `crates/core/src/netpaths.rs:130` (`unreachable_share_message`), `crates/app/src/main.rs:2300` | Complete |
| `P-73` Corrupt `games.json`/`settings.json` tolerance + atomic saves | `docs/migration/PLAN.md:237` | `crates/core/src/models.rs:563-583`, `crates/core/src/settings.rs:187-209`, `crates/core/src/json.rs:358` | Complete — the tolerance is real and matching the reference. It is also the source of `BUGS.md`'s worst finding: the tolerance is indistinguishable from an empty file, and the next write makes the loss permanent. `BUG-01` |
| `P-74` Timestamp normalization (bool/string/negative/NaN/Inf) | `docs/migration/PLAN.md:238` | `crates/core/src/json.rs` (`sanitize`, `parse_lenient`), `crates/core/src/models.rs:466-513` | Complete |
| `P-75` Unknown JSON keys ignored (forward compat) | `docs/migration/PLAN.md:239` | `crates/core/src/models.rs:192-280` (`from_dict_at` reads only known keys) | Complete |
| `P-76` Categories list with Uncategorized last | `docs/migration/PLAN.md:240` | `crates/core/src/models.rs:698-708` | Complete |
| `P-77` Duplicate game ids: last write wins | `docs/migration/PLAN.md:241` | `crates/core/src/models.rs:594-600` (`upsert`, keeping the first position) | Complete |
| `P-78` Flatpak: Gamescope extension PATH, Steam read-only path, Wine BaseApp | `docs/migration/PLAN.md:242` | `build-aux/flatpak/com.goshapps.GameHandler.json` | Complete (manifest present) — the *permission scope* is audited in `SECURITY.md` `SEC-01`/`SEC-02` |

### 3H. Non-regression (`B-01`…`B-08`) and new items (`N-01`/`N-02`)

| Feature | Where advertised | Where implemented | Status |
|---|---|---|---|
| `B-01` A wrong-typed scalar does not break the library | `docs/migration/PLAN.md:246` | `crates/core/src/models.rs:203-278` (`text`/`integer`/`flag` coercions) | Complete |
| `B-02` `added: null` does not make the sorts throw | `docs/migration/PLAN.md:247` | `crates/core/src/models.rs:626-643` (`total_cmp`, NaN-safe) | Complete |
| `B-03` `settings.json` with an invalid byte does not prevent startup | `docs/migration/PLAN.md:248` | `crates/core/src/settings.rs:187-194` | Complete |
| `B-04` A BOM'd or deeply-nested `games.json` loads instead of being emptied | `docs/migration/PLAN.md:249` | `crates/core/src/json.rs:105`, `:125-171` (`clamp_depth`) | Complete |
| `B-05` Timestamps survive a save unchanged at the bit level | `docs/migration/PLAN.md:250` | `crates/core/src/json.rs` (Python-format writer) | Complete |
| `B-06` A pathologically deep file cannot abort the process | `docs/migration/PLAN.md:251` | `crates/core/src/json.rs:125-171` (non-recursive clamp) | Complete |
| `B-07` A failed launch reports the runner's actual error text | `docs/migration/PLAN.md:252` | `crates/core/src/runners/launch.rs:102-200`, `crates/core/src/runners/mod.rs:938-947` | Complete — the port drains before reading, which is the fix |
| `B-08` Icon-vs-photo decided by bytes, never filename suffix | `docs/migration/PLAN.md:253` | `crates/app/src/view/cover.rs` (`classify`), test `:300` | Complete |
| `N-01` No-display startup diagnostic | `docs/migration/PLAN.md:264` | `crates/app/src/main.rs:376-431` (`display_present`, `no_display_hint`, `display_refusal`) | Complete — executed: bare invocation with `DISPLAY`/`WAYLAND_DISPLAY`/`WAYLAND_SOCKET` unset prints the hint and exits non-zero |
| `N-02` Explicit non-zero exit + message when the GUI cannot open | `docs/migration/PLAN.md:265` | `crates/app/src/main.rs:409` (`display_refusal`), `:432-511` (`GuiStart`, `start_gui`, `run_gui`) | Complete |

---

## 4. Controls that render but do not do what they appear to do

This is the class the brief asked for specifically. I swept it three ways: every
`Message` variant against its producers and its arm; every `on_press` /
`on_toggle` / `on_activate` in `crates/app/src/view/`; and every field on
`State`/`Settings`/`GameForm` against its readers.

**The two mechanical sweeps come back clean, and the reason is worth recording
before the list below, because it is the strongest thing this codebase does.**

- **No `Message` variant is dead.** I extracted all 65 variants from
  `crates/app/src/main.rs:567-910` and every `Message::X` construction site with
  `#[cfg(test)]` regions cut. All 65 have at least one producer, and the
  producers that live only in `main.rs` are all async replies from a worker
  thread (e.g. `Message::LaunchStarted`, `Message::RunnersRefreshed`).
- **No arm is an unmarked no-op.** `Shell::update`
  (`crates/app/src/main.rs:1640-2588`) has no wildcard arm, so a new variant is a
  compile error rather than a silent drop, and the three arms that *are* empty
  each carry a written reason (`crates/app/src/main.rs:1821` `Quit`, `:2240`
  `SetWindowHidden`, `:2584` `LaunchWatchTick`).
- **The guard that checks this is real, and it states its own blind spots.**
  `crates/app/tests/dispatch_coverage.rs` computes its covered module set rather
  than listing it, cuts `#[cfg(test)]` modules at brace depth zero, and asserts
  that its parsed arm count equals the enum's variant count — so a parser that
  stops matching fails loudly. Its module doc (`:44-105`) enumerates nine
  limits and says three of them *can* produce a false pass. That is the
  `#65` defect ("a wired-looking control that emits into an empty arm") being
  defended against by construction, and it is the correct shape.

What remains is this:

| # | Control / value | Where it renders | What actually happens | Status |
|---|---|---|---|---|
| 4.1 | `Message::RefreshPlugins` | n/a — **nothing sends it.** `crates/app/src/view/plugins.rs` has no refresh control; `crates/app/src/main.rs:2509` handles the variant and `crates/core/src/plugins.rs` is re-read on install (`crates/app/src/main.rs:2530`) | The handler works; the trigger is absent | **Parity, not a defect** — `gamehandler/qml/PluginsPage.qml` has no refresh button either, and `gamehandler/bridge.py:1003` (`refreshPlugins`) exists without a caller. However `crates/app/src/main.rs:1543-1548`'s `Message` doc block describes it as a live `T-26` handler, which reads as a wired control to anyone reading the enum. Comment accurate as of writing, stale as of now |
| 4.2 | `Message::LaunchWatchTick` | n/a — no production producer | Empty arm at `crates/app/src/main.rs:2584`; the only construction is a test decoy (`crates/app/src/view/runners.rs:2229`) | **Dead variant, documented** — `crates/app/src/main.rs:1598` and `:2684` explain that the grace watch is a timeout rather than a poll, and `crates/app/src/main.rs:2580-2583` explains why the variant is not deleted yet |
| 4.3 | `State::theme_manager: Option<()>` | n/a — never read | Written `None` at construction (`crates/app/src/state.rs:905`), declared at `:858`, and read by **nothing** in `crates/` (grep: two hits, both the declaration and the initialiser) | **Stub** — a field whose type (`Option<()>`) can carry no information. Vestigial from an earlier theme design; `theme::apply` (`crates/app/src/theme.rs:163`) is what actually changes the theme |
| 4.4 | 15 toggles on a Linux-native game | `crates/app/src/view/form.rs:286`, `:341` | 8 of the 15 disable correctly via `row_enabled` (`:678-695`, `windows_only`); the Wine-specific ones are correctly gated | Complete — checked because "a toggle that renders enabled on a Linux game" is the obvious failure mode here, and it does not occur |
| 4.5 | Settings' 13 toggles | `crates/app/src/view/settings.rs:470-480` | Each reads through `toggle_value` and writes through `toggle_selection` → `set_toggle`; every key in `DEFAULT_TOGGLES` is read and written | Complete — pinned by `every_toggle_in_the_table_reads_and_writes` (`:524`) |
| 4.6 | `Ready` / `ReleasesStatus` | `crates/app/src/view/runners.rs:212` | A real state, reached from `ReleasesFetchFinished` (`:945`) | Complete — noted only because `docs/migration/REPORT.md:56-63` records that a past audit misread this variant as a status bar that does not exist |
| 4.7 | Installers category dropdown | `crates/app/src/view/installers.rs:137` | Now chains the two real categories onto "All" | Complete — this was `T-36`'s inert dropdown; fixed |
| 4.8 | Game card context menu, Linux game | `crates/app/src/view/library.rs:534-590` | All 8 items render; the 3 prefix items render disabled | Complete |

### 4A. Where the sweep cannot reach

Two blind spots are structural and the guard documents both itself
(`crates/app/tests/dispatch_coverage.rs:96-105`):

1. **A `Message` constructed in a rendered position in `main.rs`** is not
   scanned — emissions are read from view modules only. `main.rs` production
   code *does* construct messages in its dialog builders
   (`crates/app/src/main.rs:1141` `DeleteGameConfirmed`, `:1164`
   `RemoveRunnerConfirmed`, `:1128` `CloseDialog`, `:1190` `FetchReleases`).
   I checked each by hand: all four have real handlers. The gap is live but
   currently empty.
2. **A view module nothing calls is invisible.** The installers module was the
   guard's stated example of this until `view::installers::view` was wired at
   `crates/app/src/main.rs:1401`; the doc comment at
   `crates/app/tests/dispatch_coverage.rs:87` still presents it as the live
   example and says "its page is still behind `pending_page`". **That sentence is
   stale** — `PENDING_PAGES` is empty (`crates/app/src/main.rs:3576`) and the
   module is now in the computed covered set. The comment overstates the guard's
   current blind spot in the direction that matters least, but it is wrong.

---

## 5. Feature-level gaps that are **not** defects

Recorded separately so they are not mistaken for bugs:

| # | Gap | Evidence |
|---|---|---|
| 5.1 | `P-11` advertises 9 context-menu items; both the port and the reference have 8 | `docs/migration/PLAN.md:145` vs `crates/app/src/view/library.rs:543-590` vs `gamehandler/qml/LibraryPage.qml:298-343` |
| 5.2 | `P-68`'s Qt `ApplicationShortcut` semantics (a shortcut that fires while a text field has focus) are not reproduced | `crates/app/src/shortcuts.rs` module doc; `docs/migration/REPORT.md:94-97` |
| 5.3 | `P-67`'s startup half is applied but not covered by a test | `crates/app/src/theme.rs:126-161` (the mutation table, which is honest about this); the call site itself is correct at `crates/app/src/main.rs:3717` |
| 5.4 | `--launch`'s failure text names the runner, not the game's status line, and exits 1 — a deliberate divergence | executed; `crates/app/src/main.rs:283-305` |
| 5.5 | `--list` prints `GameHandler: the library is empty` for a genuinely empty library **and** for a library it could not read | executed; `crates/app/src/main.rs:147-171`. Filed as `BUG-01`, since the CLI is a user-facing diagnostic |

---

## 6. Documentation that no longer matches the tree

Not features, but they belong in the advertised-vs-implemented contract because
each one is a place a reader is told something about the app that is not true.
Each is a one-line correction; none is a code defect.

| # | Claim | Where | What is true now | Severity of the gap |
|---|---|---|---|---|
| 6.1 | "the library pages, the pages in `PLAN.md` §4 and the ported logic land task by task — check `PLAN.md` §6 before relying on a feature here" | `README.md:110-111` | Every page is wired; `PENDING_PAGES` is `[]` (`crates/app/src/main.rs:3576`) | Tells a user to distrust a finished app. The most user-visible of these |
| 6.2 | "`main.rs:916` still routes `Page::Installers` through `pending_page(Page::Installers, "T-12")`" and "no `crates/core/src/installers.rs`" | `docs/migration/PLAN.md:348` (`T-04`'s row) | `crates/core/src/installers.rs` exists (4,265 lines) and `crates/app/src/main.rs:1401` calls `view::installers::view` | The row is a dated log with later `STATUS` entries appended, so a careful reader can date it — but the sentence is in the present tense |
| 6.3 | "`view/installers.rs` is the live example [of a module the guard does not cover]… its page is still behind `pending_page`" | `crates/app/tests/dispatch_coverage.rs:87` | Installers is in the computed covered set | Understates the guard's own coverage |
| 6.4 | `RefreshPlugins` presented as a live `T-26` handler in the `Message` doc block | `crates/app/src/main.rs:1543-1548` | The handler is live, the trigger does not exist (§4.1) | Reads as a wired control |
| 6.5 | "Detached and unreaped, exactly as Python leaves it" | `crates/core/src/runners/launch.rs:371-372` | The *unreaped* half is not Python's behaviour — `Popen` registers in `subprocess._active` and the next `Popen` reaps it; here the child stays a zombie for the session's life. See `BUGS.md` `BUG-23` | The comment is the reason a reader would not file the bug |

---

## 7. Counts

| Status | Rows |
|---|---|
| Complete | 94 |
| Partial | 10 (`P-29`, `P-35`, `P-37`, `P-38`, `P-45`, `P-49`, `P-61`, `P-68`, plus the two §4.1/§4.2 comment-accuracy items) |
| Missing | 0 |
| Stub | 1 (`State::theme_manager`, §4.3) |
| Stale documentation | 5 (§6) |

**Nothing advertised is unimplemented.** Nearly every gap is of one shape: a
feature that works on the happy path and reports or degrades wrongly on a
failure path. That is why nearly every `Partial` above has a matching entry in
`BUGS.md`, and it is the coherent story of this codebase: the *features* are
faithfully ported — the port has the reference's behaviour in front of it and
follows it closely, including into its bugs — while the *failure reporting* is
where the two diverge, sometimes deliberately better (the `B-07` drain fix) and
sometimes just missing.

**One row breaks that pattern, and it is the most important line in either
document.** `P-35`/`P-38` do not merely mis-report: the runner install cannot
complete at all. The post-extraction symlink check computes each link's parent
directory relative to the directory it is scanning rather than the candidate
root, so its escape test sees a structurally empty parent and refuses every
link that climbs — 1,818 of the 2,068 symlinks in a real Proton build
(`BUGS.md` `BUG-35`). The feature is advertised at `docs/migration/PLAN.md:179`
and `:182` as working, is drawn in the UI, downloads the archive, and then
fails with a message that blames the user's file. Everything else in this
document is a reporting or fidelity gap; this one is a broken feature, and it
is why the Runners page should not be called verified until it is fixed.
