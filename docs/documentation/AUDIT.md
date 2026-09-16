# Documentation audit — claim-by-claim fact-check

**What this file is.** The evidence record for the documentation pass
(`docs/documentation/PLAN.md`). Every user-facing or contributor-facing claim in
the repository's documentation was treated as untrusted and checked against the
current source, metadata, or scripts. Nothing here was taken from a prior report
without re-reading the cited lines.

**Method.** Read the claim, then read the thing it describes. Where a claim is a
count, the count was recomputed from the rows rather than trusted. No builds or
test suites were run for this audit; evidence is source reading plus the
project's own recorded measurements where they say what they measured. Line
numbers are as of the tree this was written against — several documents cite
older commits explicitly, and where a citation was correct at the commit named
but drifted since, that is said rather than called a lie.

**Status vocabulary.** `VERIFIED` — the claim matches the source today.
`OUTDATED` — true when written, false or misleading now. `INCORRECT` — wrong, or
contradicted by the document's own content. `MISLEADING` — literally defensible
but a reasonable reader takes the wrong meaning. `INCOMPLETE` — true as far as
it goes, missing load-bearing facts. `UNVERIFIABLE` — cannot be settled by
reading the tree. `REDUNDANT` — duplicated claim better served by one source.

**Scope note on the historical documents.** `docs/migration/` is the port's
working record: Phase-1 design docs, the task plan, the parity walk and the
findings log. `docs/audit/` is the post-port audit record, commit-scoped to
`d56782d` for its reconnaissance. These documents cite specific commits by
design, and their value is as *history*. A claim inside one that was true at its
named commit is `VERIFIED` *as history* here; it is only `OUTDATED` when the
document presents it in the present tense with no scoping, or when the document
contradicts itself.

---

## README.md

### Verified claims

The following were checked line by line against the source and hold:

| Claim | Location | Evidence |
|---|---|---|
| Current release is 0.8.0, a Python/Qt → Rust/libcosmic port | `README.md:6-10` | `Cargo.toml:18`, `meson.build:3`, `data/com.goshapps.GameHandler.metainfo.xml:68` |
| Python tree present and runnable as parity reference | `README.md:12-18` | `gamehandler/` intact; `gamehandler/__main__.py`, `main.py:121-124` |
| Front-end, not a compatibility layer; downloads from maintainers' release pages | `README.md:20-29` | `crates/core/src/runners/families.rs:76-78` (`api.github.com/repos/{github}/releases`) |
| Dark mode default plus system and light themes | `README.md:33` | `crates/core/src/settings.rs:252` (`"dark"` default), `crates/app/src/theme.rs` |
| Grid and list views, search, categories, sorting, per-game edit | `README.md:34` | `crates/app/src/view/library.rs`, `crates/core/src/models.rs` sort/search |
| Generated covers, custom covers, Steam lookup, exe-icon fallback | `README.md:35-38` | `crates/core/src/covers.rs`, `crates/core/src/exe_icons.rs` |
| Add Windows `.exe` or Linux-native games | `README.md:39` | `crates/app/src/view/form.rs:104` (`KIND_OPTIONS`, "Linux native") |
| Runner chosen at add-time, changeable later | `README.md:40` | `crates/app/src/view/form.rs` runner selection |
| Eight downloadable runner families (named list) | `README.md:41-43` | `crates/core/src/runners/families.rs` — 8 `RunnerFamily` entries + `wine-system` pseudo-runner |
| Guide describing when to use each family | `README.md:44` | per-family guide text in `families.rs`, rendered by `view/runners.rs` |
| Isolated prefixes, Winecfg, Winetricks, prefix folder | `README.md:45` | `crates/core/src/runners/` prefix tools + `view/` menu |
| Fifteen launch helpers listed by name | `README.md:46` | `crates/core/src/settings.rs:62-75` (`default_*` fields), `state.rs` `TOGGLE_NAMES` (15) |
| Nine easy installers; wizard wait then Play button | `README.md:47-48` | `crates/core/src/installers/mod.rs:290` (`INSTALLERS: [Installer; 9]`), `installers/wizard.rs` |
| Plugins page detects MangoHud, GameMode, Winetricks, UMU, Gamescope | `README.md:49` | `crates/core/src/plugins.rs:90` (`PLUGINS: &[Plugin; 5]`) |
| About & Credits page | `README.md:50` | `crates/core/src/credits.rs`, `crates/app/src/view/credits.rs` |
| Desktop shortcuts via `gamehandler --launch` | `README.md:51` | `crates/core/src/runners/desktop.rs`, `crates/app/src/cli.rs` |
| `smb://`-style shares resolved via GVFS mounts | `README.md:52-53` | `crates/core/src/netpaths.rs` |
| Gamescope needs `org.freedesktop.Platform.VulkanLayer.gamescope//25.08` | `README.md:55-59` | manifest `PATH` env at `build-aux/flatpak/com.goshapps.GameHandler.json:28`; `docs/migration/packaging.md` §3 confirms the extension name |
| Tech-stack table (edition 2024, floor 1.93, Freedesktop 25.08, software renderer) | `README.md:63-69` | `Cargo.toml:19-23`, manifest `:4-11`, `iced_tiny_skia` runtime |
| `cargo build` / `cargo run -- --version` / `--list` / bare `cargo run` | `README.md:113-118` | `crates/app/src/cli.rs:61-67`, `crates/app/src/main.rs:85-86` |
| `cargo test` + `cargo clippy --all-targets -- -D warnings` | `README.md:122-125` | `scripts/verify.sh` `test`/`clippy` stages; `[workspace.lints]` |
| Python CLI `--list` / `--launch` / `--version` | `README.md:137-161` | `gamehandler/main.py:121-124` |
| Meson build/install of the Python app | `README.md:163-170` | `meson.build`, `bin/gamehandler.in`, `data/meson.build` tests |
| Flatpak build commands and `dist/gamehandler-0.8.0.flatpak` | `README.md:177-181` | `build-aux/flatpak/build.sh:24-28,61-67` (version from Cargo.toml) |
| `--allow=multiarch` and `--filesystem=xdg-run/gvfs` rationale | `README.md:188-190` | manifest `:19`, `:25` |
| osslsigncode 2.14+ requirement and bundled trust root URL/SHA-256 | `README.md:80`, `:90`, `:192-198` | manifest `:53` (2.14 tarball), `crates/core/src/installers/signature.rs` |
| Plugins page runs no `apt`/`dnf`/`pacman`/`zypper`/`sudo`/`pkexec` inside Flatpak | `README.md:218-223` | `crates/core/src/plugins.rs:339-341` (`install_command` → `InstallError::Flatpak`), `:267-269` |
| Esync/Fsync/DXVK/VKD3D/anti-cheat on by default | `README.md:320` | `crates/core/src/settings.rs:259-266` |
| `python3 -m unittest discover -s tests -t .` | `README.md:337` | `scripts/verify.sh` `python-tests` stage uses the same invocation |
| Test-without-display paragraph (self-corrected, ARCH-08) | `README.md:350-357` | `scripts/verify.sh` unsets display vars in the pipeline; `crates/core` has no GUI dep |
| Project layout tree | `README.md:370-402` | matches the tree; `bin/gamehandler.in` exists |
| `data/` shared by both builds; trust root installed via `gamehandler/meson.build` | `README.md:404-410` | `data/meson.build`, `gamehandler/meson.build` (pem install), manifest install lines |

### Findings

**R-1. `--filesystem=home` claim — INCORRECT (highest-impact README defect).**

- Claim: "The Flatpak therefore deliberately keeps `--filesystem=home`"
  (`README.md:204-205`, with the surrounding "Reviewed game-launcher
  permissions" section `:200-208` written around the broad grant).
- Evidence: `build-aux/flatpak/com.goshapps.GameHandler.json:23-24` — the
  manifest grants `--filesystem=home:ro` plus one writable carve-out
  `--filesystem=~/.local/share/applications:create`. The narrowing is recorded
  and measured in `docs/audit/SECURITY.md` SEC-02 and `docs/audit/PLAN.md`
  (`SEC-02` row). `tests/test_packaging.py` asserts the narrowed pair **and the
  absence** of the bare `home` grant.
- Required fix: rewrite the paragraph to describe `home:ro` plus the
  applications carve-out, and say *why* read access suffices (games execute
  from home paths; the only deliberate home write is the shortcut directory).
  The `docs/migration/packaging.md` §3 citation at `README.md:216` stays correct.

**R-2. Python tree labelled "0.7.2" — OUTDATED.**

- Claim: "Python application (0.7.2, parity reference)" (`README.md:63`) and
  "the parity reference (0.7.2)" (`README.md:375`); the same `0.7.x` label
  appears in `data/meson.build:1`'s comment.
- Evidence: `gamehandler/__init__.py:3` — `__version__ = "0.8.0"`. The in-repo
  Python tree carries 0.8.0; 0.7.2 was the *released* Python version before the
  port.
- Required fix: either state the in-repo version (0.8.0) or write "the Python
  application at its last release (0.7.2)" so the number cannot be read as the
  tree's current version. Same for the `data/meson.build` comment.

**R-3. "What runs today" still describes the mid-port state — OUTDATED.**

- Claim: "the library pages, the pages in `docs/migration/PLAN.md` §4 and the
  ported logic land task by task — check `docs/migration/PLAN.md` §6 before
  relying on a feature here" (`README.md:107-111`).
- Evidence: `crates/app/src/main.rs:3432` — `PENDING_PAGES = &[]`; every page
  has a real body and a dispatch arm; `docs/audit/REPORT.md` records the port
  feature-complete against the parity checklist (120 of 128 audit findings
  fixed; remaining eight are `PARTIAL`). What legitimately remains is the
  T-19-style live verification, not the port itself.
- Required fix: rewrite the paragraph to describe the shipped state — all six
  pages wired, CLI and GUI both real — and point the "what is verified" caveat
  at the audit record rather than at the mid-port task list.

**R-4. `verify.sh` stage enumeration is a hand-maintained subset — INCOMPLETE.**

- Claim: "`scripts/verify.sh` … runs the build, clippy, the tests, the
  compatibility-oracle freshness check, the Python suite, the vendored
  `cargo-sources.json` check, the Flatpak build, a headless smoke test and the
  desktop/metainfo validation" (`README.md:127-131`).
- Evidence: `scripts/verify.sh:190-208` — the `STAGES` array has **17** stages:
  `build`, `fmt`, `clippy`, `doc`, `test`, `cli`, `oracle-freshness`,
  `python-tests`, `cargo-lock`, `advisories`, `plan-counts`, `cargo-sources`,
  `cargo-sources-fresh`, `flatpak-build`, `smoke-test`, `desktop-metainfo`,
  `flatpak-contents`. The README omits `fmt`, `doc`, `cargo-lock`,
  `advisories`, `plan-counts`, `cargo-sources-fresh` and `flatpak-contents`.
- Required fix: either list all 17 (risk: drifts again) or — better — describe
  the *categories* and send the reader to `bash scripts/verify.sh --help`, which
  prints the `STAGES` table itself. The migration doc's own lesson
  (`docs/migration/packaging.md` §6, "the list below is no longer the
  authority") applies here verbatim.

**R-5. "The build is offline" — MISLEADING.**

- Claim: "The build is offline: every crate it needs is vendored in
  `build-aux/flatpak/cargo-sources.json`, generated from `Cargo.lock`"
  (`README.md:186-188`).
- Evidence: crate *downloads* are vendored, but `build-aux/flatpak/build.sh:44-50`
  passes `--install-deps-from=flathub` by default — a network operation that
  resolves and installs the runtime/SDK — and only adds `--disable-download`
  when `GAMEHANDLER_BUILD_OFFLINE=1`. `docs/audit/PACKAGING.md` `PKG-03`
  documents that a clean checkout also needs the external cargo-sources
  generator for the freshness half.
- Required fix: scope the claim — "crate fetching is offline once the runtime
  and SDK are installed; `GAMEHANDLER_BUILD_OFFLINE=1` disables the remaining
  dependency resolution."

**R-6. "Platform" credits describe the Python stack, not the shipped app — OUTDATED/MISLEADING.**

- Claim: "### Platform — What GameHandler itself is built and shipped with"
  credits Qt, Kirigami and PySide6 (`README.md:289-298`), mirrored on the app's
  own Credits page.
- Evidence: `crates/core/src/credits.rs:278-315` — the `platform` section still
  carries only Qt/Kirigami/PySide6/Meson+Flatpak/Steam; the shipped 0.8.0 app
  is Rust + libcosmic (`iced_tiny_skia` renderer) and no libcosmic, iced or Rust
  credit exists anywhere in `CREDIT_SECTIONS`. The claim is true only of the
  parity reference.
- Required fix: add libcosmic/iced/Rust entries to the platform section and
  re-scope Qt/Kirigami/PySide6 as "the parity reference GameHandler is ported
  from". This is a shipped-product gap (the in-app Credits page) as well as a
  README one — `credits.rs` is the source of truth; the README copies it.

**R-7. Runner guide, plugin and shortcut claims — VERIFIED; no fix.**

- `README.md:44` (family guide), `:49` (five plugins incl. Gamescope) and `:51`
  (shortcuts) all match the source. The four accelerators advertised on the
  Settings page (`crates/app/src/view/settings.rs:142-146`) are wired via
  `crates/app/src/shortcuts.rs:140-141` (`listen_raw`) — including while text
  fields are focused, which the earlier audit record shows was once false and
  is no longer (`BUG-12` fixed; `main.rs:3615` subscription).

---

## Application metadata

### `data/com.goshapps.GameHandler.desktop`

Every field checked against source and build behaviour:

| Claim | Location | Verdict |
|---|---|---|
| `Name=GameHandler`, `GenericName=Game Manager` | `:3-4` | VERIFIED |
| `Comment` describes Wine/Proton + runner downloads | `:4` (comment line) | VERIFIED |
| `Exec=gamehandler` | `:5` | VERIFIED — `crates/app/src/cli.rs` is that binary |
| `Icon=com.goshapps.GameHandler` | `:6` | VERIFIED — icon installed by manifest and `data/meson.build` |
| `Categories=Game;`, `Keywords=Wine;Proton;Windows;Games;Emulation;` | `:9-10` | VERIFIED |
| `StartupWMClass=com.goshapps.GameHandler` | `:12` | VERIFIED — matches app id |
| `Terminal=false`, `Type=Application`, `StartupNotify=true` | `:7-8`, `:11` | VERIFIED |

The file passes `desktop-file-validate` in `verify.sh`'s `desktop-metainfo`
stage. No findings.

### `data/com.goshapps.GameHandler.metainfo.xml`

| Claim | Location | Verdict |
|---|---|---|
| `<id>com.goshapps.GameHandler</id>` | `:3` | VERIFIED — matches desktop file, manifest, `StartupWMClass` |
| `<name>`/`<summary>` | `:6-7` | VERIFIED |
| Description: libcosmic/Rust interface, Wine/Proton front-end | `:10-24` | VERIFIED |
| Feature bullets | `:26-35` | VERIFIED except one omission (finding M-1) |
| Keywords (lowercase per AppStream) | `:48-55` | VERIFIED — comment at `:44-47` explains the divergence from the desktop list |
| `launchable`/`provides` | `:57-60` | VERIFIED — `desktop-id` + `<binary>gamehandler</binary>` |
| homepage/bugtracker URLs | `:61-62` | VERIFIED — match `Cargo.toml` `repository` |
| Releases 0.8.0 → 0.1.0, newest dated 2026-09-11 | `:67-68`+ | VERIFIED — 11 releases, newest matches `Cargo.toml:18` |
| 0.8.0 note: "Nothing of the Python and Qt 6/Kirigami stack remains in the shipped application" | `:70-76` | VERIFIED — true of the *shipped* app; the Python tree remains in the repo as reference |
| No `<screenshots>` | — | not a defect (none exist to reference; recorded in `docs/documentation/PLAN.md` as the PKG-06 residual) |

**M-1. Helper-detection bullet omits Gamescope — INCOMPLETE.**

- Claim: "Detect optional MangoHud, GameMode, Winetricks, and UMU helpers"
  (`data/com.goshapps.GameHandler.metainfo.xml:32`).
- Evidence: `crates/core/src/plugins.rs:90` — `PLUGINS: &[Plugin; 5]`, the
  fifth being Gamescope — and `README.md:49` lists all five.
- Required fix: add Gamescope to the bullet (or drop "optional … helpers" to a
  generic phrase). Low impact; a consistency nit rather than a misstatement.

---

## Build files and user-facing comments

| File | Claim checked | Verdict |
|---|---|---|
| `Cargo.toml:1-13` | workspace layout comment (`core` GUI-free, `app` = UI + CLI) | VERIFIED — matches `crates/` |
| `Cargo.toml:18-23` | `version 0.8.0`, `rust-version 1.93` | VERIFIED |
| `meson.build:3` | `version: '0.8.0'` | VERIFIED — consistent with Cargo/metainfo |
| `data/meson.build:1-19` | comment block explaining dual Python/Flatpak installs and the LICENSE line added in `8abac0b` | VERIFIED and current — correctly describes that the Rust manifest installs the same files; the `(0.7.x)` label at `:1` is the same minor staleness as R-2 |
| `scripts/verify.sh` header | stage list lives in `STAGES`; `--help` prints it; banner checks | VERIFIED — `STAGES` at `:190-208` is authoritative and self-checking |
| `scripts/smoke-test.sh:1-70` | usage text, `--installed` precedence (PKG-01), exit codes 0/1/77 | VERIFIED — comments match the implemented build-tree-vs-installed selection |
| `scripts/ci.sh`, `scripts/parity-walk.sh`, `scripts/check-advisories.py`, `scripts/plan-counts.py` | header claims | VERIFIED by reading; `plan-counts.py` is what keeps `docs/audit/PLAN.md`'s tables honest |
| `docs/migration/oracle/gen_oracle.py`, `run_runners_vectors.py` | docstrings: deterministic fixtures, stdlib-only, import the Python reference | VERIFIED |

No findings beyond the shared `0.7.x` label noted in R-2.

---

## docs/migration/

### `PLAN.md`

**P-1. Header still says "Phase 2 in progress" — OUTDATED.**

- Claim: `docs/migration/PLAN.md:3` — "**Status: Phase 1 complete. Phase 2 in
  progress.**"
- Evidence: every code task landed (`T-38`'s row at `:390` records
  `PENDING_PAGES = &[]`; `main.rs:3432` confirms); the audit record in
  `docs/audit/REPORT.md` documents the port as feature-complete. Remaining work
  is Phase-3 verification (T-19) and the audit's eight `PARTIAL` rows.
- Required fix: update the header (e.g. "Phase 2 landed; Phase 3 verification
  outstanding") or retire the status line in favour of the audit plan.

**P-2. "The 12 decisions made so far" — OUTDATED.**

- Claim: source table at `docs/migration/PLAN.md:19`.
- Evidence: `docs/migration/DECISIONS.md` carries 57 `## D-nn` entries today.
- Required fix: drop the count or restate it as "the decision log".

**P-3. §3 deliverable sketch shows a `flatpak/` directory — OUTDATED (minor).**

- Claim: `docs/migration/PLAN.md:118` — `flatpak/  # new manifest + cargo-sources.json`.
- Evidence: the manifest lives at `build-aux/flatpak/com.goshapps.GameHandler.json`.
- Required fix: correct the path, or mark §3 as the *proposed* layout.

**P-4. Historical task prose is self-correcting but scattered — historical record, verified as such.**

- The task table (`:348-390+`) mixes pre-land claims ("T-12 … **page stays
  pinned**", `:364`; "T-29 … **PARTIALLY LANDED**", `:381`) with inline LANDED
  annotations. Each is corrected in-place, so no row is *silently* wrong — but a
  reader landing mid-document sees stale present-tense before the correction.
- Required fix: none required for correctness; if the document is rewritten for
  contributors, hoist each row's final status to the top of its cell. Recorded
  here so the pattern is named rather than re-found.

**P-5. Baseline "241 tests" — VERIFIED at baseline, stale as current.**

- Claim: `docs/migration/PLAN.md:4` — "Baseline: GameHandler 0.7.2 at
  `cbaf7e6`, 241 tests passing (1 skipped)". Same figure at
  `docs/migration/review-phase1.md:5`, `:72`, `:236`, `:638` and
  `docs/migration/DECISIONS.md:62`, `:465`.
- Evidence: the suite now holds **259** test functions across 18 files
  (`grep -rc 'def test' tests/`).
- Required fix: none — it is a baseline claim and labelled as such. Do not
  quote it as the current count.

### `REPORT.md`

- VERIFIED as a scoped record: "**Tree:** `cosmic-migration` at `a6166b8` plus
  the commits listed" (`:4-9`); its verdicts describe that tree. No
  present-tense claims that survive incorrectly today.

### `architecture.md`

- VERIFIED as design doc: `Status: design doc (Phase 1)` at `:3`, citations
  explicitly to the Python tree at its HEAD. The premise correction
  (`:9-17`) — PySide6/QML, not GTK4 — is accurate.

### `DECISIONS.md`

- VERIFIED: a decision log; entries record the state at decision time and are
  not presented as current status. D-48's `ureq`+`rustls`-in-`crates/app`-only
  note matches `crates/app/src/http.rs` holding the sole `HttpClient`.

### `packaging.md`

- Scoped Phase-1 doc (`:1-5`, "PHASE 1 documentation only"); its §3 permission
  table was updated to the narrowed grants (`home:ro` NARROWED row).
- **MP-1. §5.5 test-spec asserts `--filesystem=home` — OUTDATED.** The planned
  manifest assertions at `:522-524` list bare `--filesystem=home`; post-SEC-02
  the manifest grants `home:ro` (manifest `:23`) and `tests/test_packaging.py`
  asserts its *absence*. Fix: update the planned assertion to the narrowed
  pair.
- **MP-2. §6 stage list is explicitly non-authoritative — and its per-stage
  descriptions have still drifted.** The disclaimer at `:559-575` correctly
  defers to `STAGES`, but entry 7 (`:612-618`) describes `cargo-sources` as a
  single stage that "needs something a clean checkout does not contain" —
  post-PKG-03 the stage is split into `cargo-sources` (coverage, no external
  tool) and `cargo-sources-fresh` (`scripts/verify.sh:203-204`), and entry 6
  cites `README.md:261` for the unittest invocation (now at `README.md:337`).
  Fix: refresh the entries' descriptions, or shrink them to pointers at
  `verify.sh --help`.

### `ux.md`

- Scoped inventory at `cbaf7e6` (`:3-6`); VERIFIED as the mapping it claims to
  be. Its table rows describe the Python UI being ported from, not the Rust UI
  — no present-tense claims about `crates/`.

### `review-phase1.md`

- VERIFIED, and a model corrective document: it measures the migration brief's
  GTK4/GObject description as two releases stale (`:23-63`) and correctly
  identifies the actual target as Python 3 + PySide6 + QML/Kirigami. Its
  78-item checklist is the standard `FEATURES.md`/`T19-PARITY-WALK.md` measure
  against. Baseline counts are dated, as noted under P-5.

### `T19-PARITY-WALK.md`

- Scoped measurement at `9bcacf9` (`:4-8`, "Every claim below is labelled with
  the tree it was read from"). Its `52 (A) / 4 (B) / 22 (C)` distribution
  (`:24-36`) was accurate at that commit and is **historical**: most (C) rows
  have since been fixed (the audit record is the current status source).
- **TW-1.** Required fix, if the document stays reachable: add one line at the
  top noting the distribution describes `9bcacf9` and that later verdicts live
  in `docs/migration/REPORT.md` and `docs/audit/`. Today the header scopes the
  tree but a skimming reader can still quote 22-unmet as current.

### `VERIFY-FINDINGS.md`

- **VF-1. Header status is stale — OUTDATED.** `docs/migration/VERIFY-FINDINGS.md:8-9`:
  "**Status.** Live. Not the final report — `REPORT.md` does not exist yet, and
  the pages (T-09…T-15) are not built." Both halves are now false:
  `docs/migration/REPORT.md` exists and every page is built
  (`main.rs:3432`). The rows themselves carry per-finding fix/verify columns
  and remain accurate as a log. Fix: update the status line ("live log, kept
  for history; final report is `docs/migration/REPORT.md` and the audit
  record").

### `oracle/FINDINGS.md`, `gen_oracle.py`, `run_runners_vectors.py`

- VERIFIED: the oracle generators' docstrings describe what they do
  (deterministic, clock-frozen, stdlib-only fixtures); `verify.sh`'s
  `oracle-freshness` stage regenerates and compares them. `fixtures/` is
  excluded from this audit per scope.

---

## docs/audit/

These documents are commit-scoped to `d56782d` and carry per-row `Status:`
tails. **The tails are the source of truth** (`DECISIONS.md` D-59), and they
check out: the tail counts below were recomputed row by row from each document.

| Document | Rows | Tails observed | Matches `PLAN.md`'s derived table? |
|---|---|---|---|
| `BUGS.md` | 48 | 45 `FIXED`, 1 `PARTIAL`, 1 `CLOSED — NOT A DEFECT`, 1 withdrawn (no tail) | yes (47 tails on 48 rows) |
| `ARCHITECTURE.md` | 26 | 24 `FIXED`, 1 `PARTIAL`, 1 `WITHDRAWN` | yes |
| `COSMIC-UX.md` | 30 | 24 `FIXED`, 4 `PARTIAL`, 2 `WITHDRAWN` | yes |
| `SECURITY.md` | 11 | 11 `FIXED` | yes |
| `PACKAGING.md` | 11 | 9 `FIXED`, 2 `PARTIAL` | yes |
| `PERFORMANCE.md` | 8 | 6 `FIXED`, 2 `CLOSED` | yes |

### `PLAN.md`

- Summary tables (`:134-150`: 128 findings / 120 fixed / 8 remaining; family
  table) are **VERIFIED** — regenerated by `scripts/plan-counts.py` and guarded
  by `verify.sh`'s `plan-counts` stage (`scripts/verify.sh:203`).
- **AP-1. The tail-table footnote is stale — INCORRECT.** `PLAN.md:72-73`
  reads "`BUGS.md`'s 13 tail-less rows are its twelve open `P3` rows plus the
  withdrawn `BUG-11`". Both halves contradict the table six lines above it
  (`:64-71`: `BUGS.md` 48 rows / **47** tails → one tail-less row) and the
  document it describes (all twenty P3 rows now carry `FIXED` tails). The
  paragraph narrates its own earlier wrong revisions ("read '20 … nineteen'
  until an earlier revision…") — and the current revision is wrong again in the
  same way, which is the defect class the document is about. Fix: correct or
  delete the sentence; the derived table above it is already the right source.

### `REPORT.md`

- **VERIFIED.** The summary tables (`:60-86`: 128 found / 120 fixed / 6
  not-a-defect / 8 remaining) agree with `PLAN.md` and are produced by the same
  generator. The "What remains, honestly" section names all eight `PARTIAL`
  rows accurately, and the self-corrections are recorded rather than hidden.

### `BUGS.md`

**AB-1. The `## Counts` table contradicts the document's own rows — INCORRECT.**

- Claim: `BUGS.md:181-190` — `P0 4/4`, `P1 8/8`, `P2 14 → 12 fixed + 2
  partial`, `P3 20 → 0 fixed, 20 open`, `— 2 not-a-defect`; total **24 fixed /
  2 partial / 2 withdrawn / 20 open**.
- Evidence: recomputed from the row tails — all twenty `P3` rows
  (`BUGS.md:101-120`) end `Status: FIXED`, as do all P0/P1 and all but one P2;
  `BUG-47` is the single `PARTIAL`, `BUG-15` is `CLOSED — NOT A DEFECT` and
  `BUG-11` is the struck-through withdrawal. The true distribution is **45
  fixed / 1 partial / 2 not-a-defect / 0 open**, which is exactly what
  `docs/audit/PLAN.md`'s generated family table reports (`BUG-xx`: 46 findings,
  45 fixed, 1 remaining).
- Required fix: regenerate or delete the table. Notably, `plan-counts.py`'s
  `--check` covers only `docs/audit/PLAN.md` — the failure mode is that this
  table was hand-maintained in a document whose own thesis is that
  hand-maintained summaries drift.

**AB-2. The "P3 rows are genuinely open" paragraph — INCORRECT.**

- Claim: `BUGS.md:219-223` — "The P3 rows are genuinely open and are recorded
  as such rather than quietly closed".
- Evidence: same as AB-1 — every P3 row carries `Status: FIXED`.
- Required fix: rewrite as history ("the P3 rows were kept open until their
  tails were written") or delete.

**AB-3. Prose count "every one of the 23" — OUTDATED.**

- Claim: `BUGS.md:206-208` — "`Fixed` means … for every one of the 23, by
  restoring the pre-fix body…".
- Evidence: 45 rows are fixed; the verification standard described is still
  correct, the number is not. Fix: drop the number or recompute.

**AB-4. "Where the weight is" priority advice — OUTDATED.**

- Claim: `BUGS.md:224-232` — "`BUG-35` … Fix that first. Then
  `BUG-01`/`BUG-02` … `BUG-07`/`BUG-08`/`BUG-09` are the cheapest fixes in the
  list."
- Evidence: every row named is `FIXED`. As imperative advice to a future fixer
  it is stale; as a historical statement of what mattered most it is accurate.
- Required fix: mark it as the at-triage recommendation, or move it under a
  "what mattered" heading so it cannot read as open work.

**AB-5 (verified).** The `## Refuted` section (`:151-160`) and `## Not
verified` list (`:164-175`) are consistent: two refuted *claims* plus the
`BUG-11`/`BUG-15` rows are correctly separated, and the self-note at
`:193-203` honestly records that this table was wrong twice before.

### `FEATURES.md`

**AF-1. `P-38` Partial rationale is stale — OUTDATED.**

- Claim: `FEATURES.md:147` — `P-38` is `Partial` because `BUG-26` "is still
  open"; repeated in the counts note at `:334-336`.
- Evidence: `BUGS.md`'s `BUG-26` row carries `Status: FIXED` (containment-root
  fallback fixed — unresolvable destinations are refused).
- Required fix: re-evaluate `P-38`; per the doc's own rule (Partial ⇔ an open
  cited finding), the row should now be `Complete`.

**AF-2. `P-68` Partial rationale is stale — OUTDATED.**

- Claim: `FEATURES.md:197` — `P-68` `Partial`, "a focused `text_input` swallows
  the keys".
- Evidence: `BUG-12` is `FIXED` — `crates/app/src/shortcuts.rs:141` uses
  `listen_raw`, and `accelerator` (`:187`) no longer filters on `Status`, so
  all four accelerators fire while a field has focus.
- Required fix: re-evaluate `P-68` as `Complete`.

**AF-3. `RefreshPlugins` "trigger does not exist" — OUTDATED.**

- Claim: `FEATURES.md:259` (§4.1) and `:316` (§6.4) — nothing sends
  `Message::RefreshPlugins`.
- Evidence: `BUG-08` fixed — `crates/app/src/main.rs:1049-1050`
  (`page_entry_task`'s `Page::Plugins` arm calls `state.refresh_plugins`).
  The §4.1 row's own tail already concedes "Comment accurate as of writing,
  stale as of now" — the fix has since landed.
- Required fix: mark both entries resolved and name the fixing change.

**AF-4. §7 count line — OUTDATED as a consequence.**

- Claim: `FEATURES.md:325-329` — `Partial | 4`.
- Evidence: AF-1…AF-3 remove the cited reasons for two of the four (the
  §4.1/§4.2 comment-accuracy items were the other two; §4.1 is now also
  resolved). Required fix: recount after re-evaluating the rows.

**AF-5. §2 README citations drifted — OUTDATED (minor).**

- Claim: `FEATURES.md:59-70` cites README lines 33-58 for the feature bullets.
- Evidence: the bullets now sit at `README.md:33-53`; e.g. "Desktop shortcuts"
  is cited at `README.md:57` but is `:51`, and "GVFS" at `:58` vs `:52`. The
  claims remain true — only the addresses moved. Fix: re-cite, or cite the
  section rather than line numbers.

### `SECURITY.md`

- **VERIFIED.** All eleven rows carry `Status: FIXED` tails that match the
  current source: `home:ro` + carve-out (SEC-02), `dri`/`input`/`usb`
  (SEC-01), certificate-derived publisher subjects (SEC-11, `88419f5`),
  `env_clear` on the two security-relevant spawns (SEC-09), the `advisories`
  stage and `advisories.json` ledger (SEC-10), `https`-required runner
  downloads (SEC-05). The headline scope claim ("no untrusted data reaches a
  shell … no untrusted URL reaches `xdg-open`") is consistent with
  `SEC-07`'s recorded fix.

### `ARCHITECTURE.md`

- **VERIFIED.** 26 rows; tails 24 `FIXED` / 1 `PARTIAL` / 1 `WITHDRAWN`, all
  matching `PLAN.md`. The big structural fixes it records are real in the
  tree: `installers.rs` is now a directory module
  (`crates/core/src/installers/{mod,command,download,process,signature,tests_support,wizard}.rs`),
  `cli.rs` holds the CLI, and `main.rs` is ~11k lines with the page dispatch
  `match` covering every variant (ARCH-11/ARCH-12's `PARTIAL` is honestly
  recorded — the `State` grouping half is not done).

### `COSMIC-UX.md`

- **AU-1. Intro tail count is stale — INCORRECT.** `COSMIC-UX.md:24-25` claims
  "**Twenty** rows … seventeen `FIXED`, two `PARTIAL` and one `WITHDRAWN`".
  Recomputed: **30** rows carry tails — 24 `FIXED`, 4 `PARTIAL`, 2
  `WITHDRAWN` — matching `PLAN.md`'s derived table. The document's own next
  paragraph records this sentence being wrong before; it drifted again. Fix:
  point at `plan-counts.py --check` rather than restating a number.
- Row-level content otherwise reads consistently with the tails; the two
  `WITHDRAWN` rows (`UX-08`, `UX-21`) are correctly marked and refuted-by-
  measurement.

### `PACKAGING.md`

- **VERIFIED.** 11 rows; tails 9 `FIXED` / 2 `PARTIAL` (`PKG-03`'s
  generator-vendoring half and `PKG-06`'s screenshots residual are honestly
  `PARTIAL`). `PKG-01` (smoke test prefers the build tree) and `PKG-08`
  (LICENSE now installed by the manifest) match `scripts/smoke-test.sh` and
  the manifest respectively.

### `PERFORMANCE.md`

- **VERIFIED.** 8 rows; 6 `FIXED`, 2 `CLOSED` (`PERF-07`/`PERF-08` closed as
  not-a-defect on re-measurement). The header's disclosure that four rows'
  citations were wrong *when written* is exactly the candour this audit
  checks for — recorded, not a defect to fix.

### `DECISIONS.md`

- **VERIFIED.** D-57…D-63 record the audit's own conventions; D-59's
  description of the tail landscape is explicitly framed as historical ("At
  the time this was decided…") and correctly so.

### `BASELINE.md`

- **VERIFIED as a dated snapshot.** "Run on `audit-hardening` at `eb2c47f`:
  all stages green" (`:42-49`) is a recorded measurement of that commit's
  twelve-stage script. It must not be quoted as the current state:
  `scripts/verify.sh` has since grown from 2,125 to 2,747 lines and from 12
  to 17 stages. No fix required — the commit pin is present — but cite it as
  "at `eb2c47f`", never as "verify.sh is green".

### `advisories.json`

- **VERIFIED.** The ledger is what `check-advisories.py` compares against
  `Cargo.lock` in the `advisories` stage, and its header honestly states the
  limits (offline, frozen clone date, transitive pins it cannot fix).

---

## docs/documentation/PLAN.md

- **VERIFIED and consistent with this audit.** Its verdict table
  (`README.md` → REWRITE, metainfo/desktop → KEEP, migration/audit → KEEP as
  engineering record) matches every finding above, including the README's
  `home:ro` discrepancy, the `0.7.2` staleness, the verify.sh enumeration and
  the missing `<screenshots>` residual it already names. This file is the
  `AUDIT.md` it anticipates.

---

## Cross-cutting finding: the audit corpus's own dominant defect

The migration/audit record names the repository's characteristic defect —
*the check or summary that does not inspect what it claims* — and the same
shape recurs inside the documentation itself, in three live instances found by
recomputation rather than reading:

1. `docs/audit/BUGS.md`'s `## Counts` table and "P3 rows are open" prose
   contradict its own 45 `FIXED` tails (AB-1, AB-2).
2. `docs/audit/PLAN.md:72` narrates a tail-less count that contradicts the
   derived table directly above it (AP-1).
3. `docs/audit/COSMIC-UX.md:24-25` restates a tail count that is wrong again
   after being corrected once (AU-1).

`scripts/plan-counts.py` guards `docs/audit/PLAN.md`'s *tables*; the unguarded
surfaces are every other document's *prose* numbers and `BUGS.md`'s table.
If the corpus is kept, the cheap fix is to extend the check or delete the
hand-maintained numbers.

A second, milder pattern: **`file:line` citations drift.** Several audit rows
cite paths that moved after `installers.rs` was split (ARCH-11) and the
README's feature lines shifted (AF-5). Most documents are commit-scoped so
this is expected; where a document is meant to be read as current, prefer
symbol names over bare line numbers.

---

## Prioritised must-fix list (by user impact)

1. **`README.md:204-205` — `--filesystem=home` → `home:ro` + carve-out (R-1).**
   The permission section is what a security-conscious user actually reads;
   it currently documents a grant the Flatpak no longer has, in the direction
   that *overstates* exposure. Highest impact by far.
2. **`crates/core/src/credits.rs:278-315` + `README.md:289-298` — platform
   credits name Qt/Kirigami/PySide6 and omit libcosmic/Rust (R-6).** This is
   user-visible *in the shipped app*, not just in docs: the Credits page
   describes the wrong toolkit.
3. **`README.md:63`/`:375` — Python "0.7.2" label vs `__version__ = "0.8.0"`
   (R-2).** Version confusion about which tree is which is the README's
   core job to get right.
4. **`README.md:107-111` — "lands task by task" mid-port framing (R-3).**
   Undersells the shipped app; a new user may believe the GUI is unfinished.
5. **`README.md:127-131` — incomplete `verify.sh` stage list (R-4).** Fix by
   pointing at `--help` so it cannot drift again.
6. **`docs/audit/BUGS.md:181-190`, `:205-223` — counts table and open-P3
   prose (AB-1…AB-3).** Internal, but the audit record is what contributors
   consult for "what is left"; today it says 20 bugs are open when none are.
7. **`docs/audit/FEATURES.md` §7/`P-38`/`P-68`/`§4.1` (AF-1…AF-4)** — same
   class: rows say `Partial` against fixed bugs.
8. **`docs/audit/PLAN.md:72` and `docs/audit/COSMIC-UX.md:24-25` (AP-1,
   AU-1)** — self-contradicting prose counts; small fixes, high irony value.
9. **`docs/migration/VERIFY-FINDINGS.md:8-9` (VF-1) and
   `docs/migration/PLAN.md:3`/`:19` (P-1, P-2)** — stale status headers on
   the working record; fix or mark the files historical.
10. **`data/com.goshapps.GameHandler.metainfo.xml:32` (M-1),
    `README.md:186` offline claim (R-5), `docs/migration/packaging.md` §5.5/§6
    entries (MP-1, MP-2), `FEATURES.md` README citations (AF-5),
    `T19-PARITY-WALK.md` top-note (TW-1)** — low-impact consistency fixes.

---

## Items verified clean (no action)

- All 8 runner families and their upstream endpoints
  (`crates/core/src/runners/families.rs`); GitHub releases API + homepage URL
  derivation.
- All 9 easy installers, their HTTPS URLs, `allowed_hosts` redirect
  validation, and Authenticode verification incl. the pinned Microsoft
  Identity Verification Root CA 2020 (`installers/mod.rs:290`,
  `installers/download.rs`, `installers/signature.rs`, manifest `:53`).
- XDG/`GAMEHANDLER_*` path behaviour (`crates/core/src/paths.rs`) and all
  settings defaults (`crates/core/src/settings.rs:244-272`).
- The five-plugin catalog and Flatpak detection rules
  (`crates/core/src/plugins.rs`), incl. no package-manager commands inside
  the sandbox.
- Keyboard shortcuts: four accelerators, `listen_raw`, libcosmic's own
  `Ctrl+F` path (`crates/app/src/shortcuts.rs`,
  `crates/app/src/view/settings.rs:142-146`).
- CLI: `--list`/`--launch <id>`/`--version` dispatched before any GUI
  initialisation (`crates/app/src/main.rs:85-86`, `crates/app/src/cli.rs`).
- All Rust UI pages exist and are dispatched; `PENDING_PAGES` is empty
  (`crates/app/src/main.rs:3432`).
- Desktop entry and metainfo fields, links, releases and version lockstep
  (Cargo `0.8.0` = meson `0.8.0` = metainfo newest release).
- Version consistency across `Cargo.toml`, `meson.build`,
  `gamehandler/__init__.py`, metainfo, and the bundle name
  `dist/gamehandler-0.8.0.flatpak`.
