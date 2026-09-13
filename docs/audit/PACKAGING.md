# Packaging, platform and QA audit

## Scope

This is an audit of how GameHandler is packaged, distributed and verified,
performed by reading the tree at commit `d56782d`. The reconnaissance pass was
read-only; the fixes it produced landed separately on branch `audit-hardening`
and each row below carries its own **Status** tail. The six areas examined were,
in order: the Flatpak manifest
(`build-aux/flatpak/com.goshapps.GameHandler.json`) and the scripts under
`scripts/` and `build-aux/` (correctness, completeness, reproducibility, base
and runtime versions, and whether `cargo-sources.json` is fresh and covers every
git dependency); desktop integration (`.desktop` and AppStream metainfo
validity, completeness, categories, MIME types, keywords, and icons at the sizes
a consumer expects); version consistency across `Cargo.toml`, the metainfo
release entries, the desktop file and `--version` output; build-process
reproducibility (hardcoded paths, machine-specific assumptions); test coverage of
packaging; and licensing consistency.

Every claim cites a `file:line` I read or a command I ran, with its **actual**
output. The requested validators were run and their output is reported verbatim,
**warnings included**: a validator that exits 0 while printing a warning is
reported here as having printed a warning. Where something could not be settled,
it is in **Not verified** with the test that would settle it.

**Headline results.** The manifest itself is in good shape: `flatpak-builder
--show-manifest` parses it (exit 0), every one of the 11 git dependencies in
`Cargo.lock` has a matching `type: git` entry in `cargo-sources.json`, all 626
crates.io and 11 git sources are pinned by `sha256`/commit, the build runs with
`CARGO_NET_OFFLINE=true` and `--locked`, versions agree across all six places
they appear, and the application's own `LICENSE` **is** installed and
byte-identical in the built tree. The problems are in the verification chain
around the artefact rather than in the artefact: **nothing in this repository
runs the verification scripts automatically** (no CI, no git hooks, no
make/just entry point, and the only `meson test` entries belong to the retired
Python tree), and the one stage that actually executes a binary can silently
execute a **different, previously installed build** rather than the one just
produced — which I confirmed by hash.

## Findings

Sorted most severe first.

| ID | Severity | Finding | Evidence (`file:line` or command + output) | Impact | Recommendation |
|---|---|---|---|---|---|
| PKG-01 | P1 | **The only check that executes the built artefact can execute a stale, previously installed build instead.** `smoke-test.sh` decides once, before any check, whether to test the installed app or the build tree — and it prefers the installed app: `if flatpak info "$APP_ID" >/dev/null 2>&1; then RUN_MODE="installed"; elif [ -d "$BUILD_DIR/files" ]; then RUN_MODE="build-dir"`. On a machine where any older GameHandler is installed in the user installation, the smoke stage exercises **that** binary and reports green for a build it never ran. This matters because `verify.sh`'s `flatpak-contents` stage never executes anything (see PKG-04), so `smoke-test` carries the entire "the artefact boots" burden. | `scripts/smoke-test.sh:159-163`; run mode consumed at `:189-219`. **Verified on this machine**: the installed app is a different binary from the tree just built — installed `/home/gosh/.local/share/flatpak/app/com.goshapps.GameHandler/x86_64/stable/db7b22fd…/files/bin/gamehandler` is `29458152` bytes, `sha256` starts `468519308634657f`; the freshly built `build-flatpak/files/bin/gamehandler` is the same `29458152` bytes but `sha256` starts `801fb8094a74ec2a` — **different artefacts, identical size and identical `--version` output**. `flatpak info com.goshapps.GameHandler` reports `Commit: db7b22fddbc6925acc8c1dcfc069afbf91d72e205cca8dda7a1bd33095187930`, `Date: 2026-09-12 17:56:44 +0000`. | A packaging regression introduced by the tree under test can pass the smoke stage: the stage reports `ok "gui-stays-up …"` for a binary that is not the one `flatpak-build` produced. This is precisely the "passes without inspecting what it claims" defect class the project's own notes name as recurring, one layer out from the code. | Invert the preference when a build tree exists (use `BUILD_DIR` when `-d "$BUILD_DIR/files"`, and treat `installed` as the fallback), or record and print the `sha256` of the binary actually executed and assert it equals the one in the build tree. Printing the hash alone would have made this visible without changing the test's shape. **Status: FIXED `8abac0b`.** `scripts/smoke-test.sh` now prefers the build tree and names what it ran. The preference is inverted and the inversion is deliberate: when `-d "$BUILD_DIR/files"` holds a `bin/` the tree wins, and `installed` is the fallback rather than the default. A new `subject:` banner line prints the path and `sha256` of the binary actually executed, and a `--installed` flag was added for the case where the installed app *is* the subject. Measured both ways. Before — with a populated build tree present and the app installed — the script chose `mode=installed`, executing the installed `4685193…` binary, a different artefact from the tree it was asked to test. After — `mode=build-dir`, `subject: … 81db31bc…`. The unfinished-tree case is the one a clean checkout meets: with `files/` present but holding no `bin/gamehandler`, the old script fell through to the installed app and exited `77` — a skip, which the chain does not report as a failure — while the fixed script says the tree holds no binary and exits `1`. The new `APP_CMD_GIVEN` guard keeps the `--app-cmd` self-test hook (which deliberately points at `/app/bin/7z`) from tripping that same check. |
| PKG-02 | P2 | **Nothing in the repository runs the verification chain automatically.** There is no CI configuration, no installed git hook, and no make/just entry point; the only `test()` definitions in the tree belong to the retired Python application and are reachable only via `meson test`, which nothing invokes. `scripts/verify.sh` (2,125 lines, 12 stages) and `scripts/parity-walk.sh` (753 lines) are hand-invoked only; `verify.sh` calls `smoke-test.sh` and nothing calls `verify.sh`. | Enumerated directly, not inferred. `ls -a1` at the repo root lists `bin build-aux build-dir build-flatpak Cargo.lock Cargo.toml crates data dist docs .flatpak-builder flatpak-repo gamehandler .git .gitattributes .gitignore LICENSE meson.build README.md rust-toolchain.toml scripts target tests` — **no `.github/`**, and `find . -maxdepth 3 \( -name '.github' -o -name '*.yml' -o -name '*.yaml' -o -name 'Jenkinsfile' \)` outside `.git`/`target`/`build-flatpak`/`.flatpak-builder` returns **nothing**. `ls -a1 .git/hooks/` shows only `.` and `..` after filtering `*.sample` → **no hooks installed**; `git config --get core.hooksPath` exits 1 (unset). The only `test()` calls are `data/meson.build:38-41` (`validate-desktop`) and `:50-57` (`validate-metainfo`), in a file whose own header says it is the Python application's and that "The Rust application (0.8.0) does NOT use Meson" (`:1-6`). `grep -rn "parity-walk.sh"` outside `docs/` returns nothing. | A packaging regression is caught only when a human remembers to run a 2,125-line script. The audit trail this project relies on (walk verdicts, REPORT, gate fixes) is therefore dependent on memory, and there is no artefact — no CI log, no hook output — that shows the chain ran against a given commit. | Add the cheapest thing that makes the chain non-optional: a `pre-push` hook (or a CI workflow) running at minimum `verify.sh` stages `build`, `clippy`, `test`, `desktop-metainfo` and `cargo-sources`, with `--skip-flatpak` for the slow ones. The stage machinery, exit codes and locking are already built and correct; they need a caller. **Status: FIXED `372b86e`.** `scripts/ci.sh` is that caller, and it is deliberately thin: it picks the flag set, records the commit and branch it saw, tees the run to `target/ci/<timestamp>-<sha12>.log`, and exits with `verify.sh`'s own status. It holds no stage list of its own — a second copy of the stage list is exactly the drift the `STAGES` array exists to prevent (tasks #29 and #48, both of which read green). `--full` drops `--skip-flatpak`; `--keep-going` passes through. Verified against a stub first: exit codes `0/1/2/3/7` propagate unchanged, each with its matching verdict line; the argument vector is `--skip-flatpak` by default and drops it under `--full`, with `--keep-going` appended in both cases; the log is written and carries the commit, branch, dirty flag and full output. Then run for real — and it caught a genuine `test` failure in this very tree (the stale ARCH-05 line citations, fixed in `75fc739`), exiting `1` with `FAIL test` named. That is the row's own "break something and watch it fail" check, satisfied by accident rather than by construction. No `.github/workflows` is added, and the reason is written in the file: the chain needs `flatpak-builder`, the `org.freedesktop.Sdk` 25.08 runtime and that SDK's rust-stable extension, and this tree records the host package set nowhere, so a workflow authored here would guess at a runner image and red on its first run for a reason that is not the code. Exit `3` is called out by name in the script's output, because it is the status a reader mistakes for green: a stage skipped for a missing prerequisite means the run did not verify what its stage list claims. The `pre-push` hook is documented rather than installed — this is a shared checkout, and `.git/hooks` is repository state a verification script should not change behind the other sessions using it. |
| PKG-03 | P2 | **Both the Flatpak build and the `cargo-sources` freshness check depend on things a clean checkout does not contain, and one of them needs the network by default.** `flatpak-builder` is invoked with `--install-deps-from=flathub` unconditionally; the `--offline` flag does not remove that, it only adds `--disable-download`. The freshness check needs `flatpak-cargo-generator.py` — which is **not in the repository** — plus a Python that can `import aiohttp`; and because 11 dependencies are git sources, regenerating on a cold `~/.cache/flatpak-cargo` runs `git clone`/`git fetch`. | `scripts/verify.sh:1346` (`--install-deps-from=flathub`), `:1351-1353` (the `--offline` note; `--disable-download` is *added*, not substituted), `:1210-1226` (`find_cargo_generator`: env var → `command -v` → the hardcoded candidates `"$HOME/.cache/flatpak-builder-tools/cargo/flatpak-cargo-generator.py"`, `/tmp/gh-gen/flatpak-cargo-generator.py`, `/usr/share/flatpak-builder-tools/cargo/flatpak-cargo-generator.py`), `:1230-1240` (`find_generator_python`, which requires `import aiohttp`), `:1290`/`:1298` (`return 99` when either is absent). Repo check: `git ls-files build-aux` lists only `build.sh`, `cargo-sources.json` and the manifest — no generator. `build-aux/flatpak/build.sh:34-42` carries the same unconditional `--install-deps-from=flathub`. | The packaging build is not reproducible from a clean checkout without network access and two pieces of tooling that live outside the repo (one of them, `/tmp/gh-gen/…`, at a path that is not durable). At least the failure is honest — a missing prerequisite returns `99`, which maps to an unrequested skip and **exit 3**, not exit 0 (`verify.sh:1397-1398`, `:2118-2123`) — but a fresh contributor cannot get a green run. | Vendor the generator (it is a single Python file) under `build-aux/flatpak/`, and document the aiohttp requirement in the README rather than in a comment. Consider `--install-deps-from` being conditional on a flag so that a fully-provisioned machine can build with `--disable-download` and no network. |
| PKG-04 | P2 | **`flatpak-contents` verifies placement and bytes but never runs the binary, and its one binary check is a text search inside the ELF rather than an execution.** The stage asserts the five non-binary installs against a declared path list, asserts the manifest declares each install, asserts nothing is installed that the list omits, and (when a build tree exists) `cmp`s each installed file against the repo copy. For the binary itself it only `grep -a`s the ELF for placeholder strings. There is **no `flatpak run`, `flatpak build`, `flatpak install`, `flatpak info` or `flatpak kill` anywhere in `verify.sh`**. | `scripts/verify.sh:1544-1550` (`FLATPAK_CONTENTS`, the five entries), `:1568-1571` (repo-side presence), `:1573` (`DECLARES_PY`, defined `:1475-1496`, requiring the command to begin with `install `), `:1558` (completeness, `COMPLETENESS_PY`), `:1763-1773` (`[ ! -f ]` → `[ ! -s ]` → `cmp -s`), `:1693-1756` (the ELF placeholder cross-check, with `grep -ao -- 'This page has not been ported yet ('` at `:1704-1705`). `grep -n "flatpak run\|flatpak build\|flatpak install\|flatpak info\|flatpak kill" scripts/verify.sh` → **no matches**. The stage's own comment concedes its tree detection is a marker-based inference rather than mtime (`:1597-1603`). | The chain can prove a file landed at the right path with the right bytes and still ship a binary that does not start — the sandbox is where a missing runtime, a bad `Exec`, an unresolved `.so` or a wrong `base` shows up, and none of that is visible to `cmp`. With PKG-01, the only execution evidence available points at a possibly-different installed build. | Add one `flatpak run --command=gamehandler com.goshapps.GameHandler --version` (and `--list`) against the **freshly built** ref before the bundle is produced, and assert the exit code and the version string. That is the same assertion smoke-test already makes, pointed at the right artefact. **Status: FIXED `ead99e7`.** The stage now executes the artefact it has just verified the bytes of. It runs `flatpak build --unset-env=DISPLAY --unset-env=WAYLAND_DISPLAY --unset-env=WAYLAND_SOCKET "$BUILD_DIR" /app/bin/gamehandler --version`, asserts exit `0` and `GameHandler <major>.<minor>.<patch>` on stdout, then repeats for `--list` with exit `0`, and prints the artefact's `sha256` on success so the executed bytes are named; a missing `flatpak` returns `99` like the rest of the stage. Measured in both directions rather than asserted: a healthy tree gives `GameHandler 0.8.0` and exit `0`; with the binary made non-executable by `chmod -x`, the same invocation gives `bwrap: execvp /app/bin/gamehandler: Permission denied` and exit `1` — the sandbox failing in exactly the way `cmp` cannot see. A control against the pre-fix revision confirmed the claim being fixed: `grep` for `flatpak run|build|install|info|kill` in the old stage body returned nothing, so nothing in it had ever executed anything. One measured detail worth recording, because it is a trap for anyone extending this stage: `flatpak build <dir>` needs the build root that has `metadata` beside `files`; passing the `files/` subdirectory itself gives `error: Build directory … not initialized, use flatpak build-init`. The stage reuses the `$BUILD_DIR` it already computed for that reason. |
| PKG-05 | P2 | **The application ships a single 128×128 SVG and no PNG icon at any size.** The only icon file in the tree is `data/icons/hicolor/scalable/apps/com.goshapps.GameHandler.svg`, installed to the same hicolor path, and the export directory of the installed app contains exactly that one file. The metainfo has no `<icon>` element, so there is no AppStream-level icon either. | `data/icons/hicolor/scalable/apps/com.goshapps.GameHandler.svg` — SVG header reads `width="128" height="128" … viewBox="0 0 128 128"`, so the master artwork is 128 px. Installed by `build-aux/flatpak/com.goshapps.GameHandler.json:93`. Export contents: `find ~/.local/share/flatpak/app/com.goshapps.GameHandler/current/active/export/share/icons -type f` → exactly `…/hicolor/scalable/apps/com.goshapps.GameHandler.svg`. `grep -n "<icon" data/com.goshapps.GameHandler.metainfo.xml` → no matches. | Every consumer that cannot rasterize SVG — some launchers, docks, notification daemons, and the Flathub submission checks, which expect at least a 128×128 PNG and conventionally 64/128/256/512 — has no icon to draw, and the app appears as a generic placeholder there. This row also asserted that "because the master is only 128 px, a 512 px PNG cannot be produced without new artwork"; **that was wrong**, and the correction is below rather than in a rewrite. | Export PNGs at 64/128/256 (and 512 if the master is redrawn larger) into `data/icons/hicolor/<size>x<size>/apps/` and install them the way the SVG is installed; optionally add an `<icon type="stock">` to the metainfo. The SVG can stay for the environments that prefer it. **Status: FIXED `9e10566`.** 64/128/256/512 PNGs are rendered from the SVG and installed from both manifests — the Flatpak's (`build-aux/flatpak/com.goshapps.GameHandler.json`) and the meson tree's (`data/meson.build`) — and the metainfo gained `<icon type="stock">`. The `128 px` reading is a *raster* bound and not an artwork bound: the SVG is 820 bytes of `<rect>`/`<circle>`/`<path>` elements with no `<image>` and no embedded raster, so it scales without loss and `rsvg-convert -w 512 -h 512` produces a genuine 512×512. Measured on the four checked-in PNGs by decoding each one's `IHDR` and its `IDAT` (not by opening them): dimensions 64/128/256/512, colour type 6 (RGBA), bit depth 8, non-interlaced, opaque coverage 71.7/72.4/72.7/72.9 % and 209/377/664/1126 distinct pixel values — a rendered image at every size and not a blank or an upscale. Re-rendering 256 twice gives the same `sha256` (`3c4ac45e…`), so the checked-in bytes are reproducible from the checked-in SVG. |
| PKG-06 | P3 | **The metainfo has no screenshots and no keywords.** Both are optional in the AppStream specification, which is why the validator passes — but a screenshot is required for an app to be presented properly in a software centre, and keywords are what AppStream search matches on. The `.desktop` file has keywords; the metainfo does not, so the two disagree about how the app is described. | `grep -n "<screenshots\|<screenshot\|<keywords" data/com.goshapps.GameHandler.metainfo.xml` → **no matches**. The desktop file does carry them: `data/com.goshapps.GameHandler.desktop:10` `Keywords=Wine;Proton;Windows;Games;Emulation;`. | Reduced discoverability in software centres and a listing that renders without imagery. The validator is clean, so nothing will prompt anyone to add these. | Add one or two `<screenshot type="default">` entries pointing at images hosted with the project (they must be reachable over HTTPS — not verified here) and a `<keywords>` block mirroring the desktop file's. |
| PKG-07 | P3 | **The AppStream validator is not clean under `--pedantic`: it prints a warning while exiting 0, and the validator the project's own meson test names could not be run at all.** Reported verbatim rather than summarised as a clean run. | `appstreamcli --version` → `AppStream version: 1.1.3`. `appstreamcli validate --no-net data/com.goshapps.GameHandler.metainfo.xml` → `✔ Validation was successful: pedantic: 1`, exit 0. `appstreamcli validate --no-net --pedantic …` → prints `P: com.goshapps.GameHandler:3: cid-contains-uppercase-letter com.goshapps.GameHandler`, then `✔ Validation was successful: pedantic: 1`, exit 0. `appstream-util` is **not installed** in this environment (`command -v appstream-util` → not found), so the alternative binary that `data/meson.build:50` also accepts (`find_program('appstreamcli', 'appstream-util')`) could not be exercised. `desktop-file-validate data/com.goshapps.GameHandler.desktop` → **no output, exit 0**. | The pedantic warning is about the reverse-DNS component id `com.goshapps.GameHandler` containing uppercase letters. It is informational under this validator's own rules and does not fail anything here, but a stricter linter (Flathub's own) is entitled to treat it as a defect, and the id is embedded in the desktop file name, the metainfo filename, `StartupWMClass`, the Flatpak app-id and the export paths, so changing it later is expensive. Recording it now is the cheap option. | Decide deliberately whether the uppercase component id is intended (the app-id and the metainfo id must stay identical, so this is a one-way decision), and record the answer. If the warning is accepted, say so in the packaging notes so a future Flathub submission is not a surprise. |
| PKG-08 | P3 | **A comment in `data/meson.build` asserts a packaging regression that does not exist, and its cited evidence is falsified by the manifest.** The comment states that the LICENSE install "has no counterpart in the Rust manifest, which installs only the osslsigncode and DXVK licence texts", asserts that `grep LICENSE` over the manifest "finds no source that is the application's own COPYING, and no `"LICENSE"` path at all", and concludes "This is a live regression, not a hypothetical". The manifest installs the application's own LICENSE at line 94, and the built tree contains it byte-identical. | The stale claim: `data/meson.build:13-23` (the quoted sentences are at `:14-17` and `:18`). The counter-evidence: `build-aux/flatpak/com.goshapps.GameHandler.json:94` — `"install -Dm0644 LICENSE ${FLATPAK_DEST}/share/licenses/com.goshapps.GameHandler/LICENSE"`; and the built tree, `ls -la build-flatpak/files/share/licenses/com.goshapps.GameHandler/` → `LICENSE` (35,149 bytes), `DXVK.txt` (1,074), `osslsigncode-LICENSE.txt` (1,530); `cmp LICENSE build-flatpak/files/share/licenses/com.goshapps.GameHandler/LICENSE` → **byte-identical**. The same comment's other citation, `…json:91-93` for the three metadata installs, is still accurate (`:91-93` are exactly those three `install -Dm0644` lines), which is what makes the stale half easy to believe. | Anyone reading this file to decide whether the licence is shipped is told, with an emphatic and specific claim of verification, the opposite of the truth. The cost is asymmetric: believing it leads to "fixing" something that is already correct — or, worse, to deleting the meson `install_data` on the strength of a comment that is wrong about which direction the gap runs. | Delete or correct `data/meson.build:13-23`. The file is the Python tree's and the whole block could go with it when that tree is retired (see PKG-02), but while it stays it should not assert something a one-line grep contradicts. |
| PKG-09 | P3 | **`flatpak-contents` and `cargo-sources` both shell out to a bare `python3` for their comparison helpers, independently of the interpreter chosen to run the generator.** The generator is run with a discovered interpreter (`$py`, the one that can `import aiohttp`), but the two inline comparison scripts are invoked with `python3` from `PATH`. On a machine where the aiohttp-capable interpreter is not the default one, the generator runs under one Python and the comparisons under another. | `scripts/verify.sh:1313` (`python3 -c "$COMPARE_PY" …`), `:1326` (the git-coverage helper), against `$py` usage at `:1305`; `find_generator_python` at `:1230-1240` is what selects `$py`. | Low: both helpers are pure `json` and `re` over strings, so a version skew is unlikely to change a result today. It is a latent trap — the day a helper uses a feature that differs, the failure will look like a stale `cargo-sources.json` rather than a wrong interpreter. | Use `"$py"` for the helpers too, or assert that `python3` and `$py` are the same interpreter at stage entry. |

| PKG-10 | P2 | **The checked-in `Cargo.lock` does not match the manifest at `d56782d`: it lists `iced_accessibility` as a dependency of `gamehandler`, and `crates/app/Cargo.toml` does not declare it — so `cargo metadata --locked` fails on a clean checkout of the audited commit, and nothing in the tree notices.** Found by running the command rather than by reading the files: the two disagree and only executing cargo settles it. | `git show be31a7b -- crates/app/Cargo.toml` — that commit's message says "`iced_accessibility` comes from the same libcosmic repository at the same rev, for the same reason", and its `Cargo.lock` hunk adds `"iced_accessibility",` to the `gamehandler` package's dependency list, but `git show be31a7b:crates/app/Cargo.toml` contains **zero** occurrences of the name. Measured on the tree as it stood: `cargo metadata --locked` exits 101 (`cannot update the lock file … because --locked was passed`), while `cargo metadata --offline` exits 0 — so the break is real and `--offline` is what made it invisible. | Root cause is a partial commit, and it is the general form of one: `be31a7b` staged a `Cargo.lock` that reflected a *concurrent* uncommitted change in the same working tree, so the lockfile committed a dependency the commit's own manifest did not have. `Cargo.lock` is generated, so it is exactly the file where a partial commit silently records work the commit does not contain — and because generated files are read as derived, a reader has no reason to compare it against its input. Impact is a build-integrity failure rather than a runtime one: a consumer who builds with `--locked` (the flag a downstream packager or a reproducible build uses) is refused, and the refusal names the lock rather than the cause. | A `cargo-lock` stage in `scripts/verify.sh` running `cargo metadata --offline --locked --format-version 1`, and the missing declaration restored. **Status: FIXED.** The declaration was restored in the commit that carried the accessibility work, which is the same uncommitted change whose dependency `be31a7b` had swept into the lock — so the file and its input agree again, though for a reason nobody chose. The stage is the part that matters: it is the check that would have caught this, and it was proved able to fail by dropping a line from `gamehandler`'s dependency list in the lock and watching the stage exit 101, then restoring it and watching it exit 0. `--offline` is deliberate: the check is about the two files agreeing, so it must not need the network, and `--locked` is what makes cargo compare instead of rewrite. |

## Verified sound

1. **The manifest is well-formed and parseable.** `flatpak-builder
   --show-manifest build-aux/flatpak/com.goshapps.GameHandler.json` → exit 0,
   emitting the fully-expanded manifest. Top-level: `app-id
   com.goshapps.GameHandler`, `branch stable`, `command gamehandler`, `runtime
   org.freedesktop.Platform` `25.08`, `sdk org.freedesktop.Sdk`,
   `sdk-extensions ["org.freedesktop.Sdk.Extension.rust-stable"]`, `base
   org.winehq.Wine` `base-version stable-25.08`, `inherit-extensions
   ["org.freedesktop.Platform.GL32", "org.freedesktop.Platform.Compat.i386"]`,
   `cleanup ["/include", "/lib/cmake", "/lib/pkgconfig", "/share/doc"]`. The
   `base-version` (`stable-25.08`) is consistent with `runtime-version` (`25.08`),
   which is the pairing a Wine base requires.
2. **Every dependency is pinned, and the pins are complete.** All 11 git
   dependencies in `Cargo.lock` have a matching `type: "git"` entry in
   `cargo-sources.json`, each pinned to a full 40-character commit — I checked
   this myself rather than trusting the project's script:
   `python3` over `Cargo.lock`'s `^source = "git\+…` lines yields **11 urls, 11
   OK, 0 missing**, against **11 `type: git` entries** in the 1,417-entry
   `cargo-sources.json` (479,422 bytes: 729 `inline`, 626 `archive`, 51 `shell`,
   11 `git`). The apparent earlier discrepancy is trailing-`.git` normalisation
   (`libcosmic.git` in the lock versus `libcosmic` in the JSON) and is not a
   real gap. The 626 crates.io archives each carry a `sha256`, and the 11 git
   entries each carry a commit.
3. **The build is offline by construction.** `CARGO_NET_OFFLINE=true` and
   `append-path /usr/lib/sdk/rust-stable/bin` in `build-options`, with `cargo
   --offline fetch --locked --manifest-path Cargo.toml --verbose` followed by
   `cargo --offline build --locked --release` — so the vendored sources must
   resolve or the build fails, which is exactly the property that makes
   `cargo-sources.json` load-bearing rather than decorative. (The Flatpak-level
   network dependency is a separate matter — PKG-03.)
4. **The installed tree is correct, and the export is correct.** Verified in the
   built tree and against the installed app rather than from the manifest alone.
   `build-flatpak/files/share/licenses/com.goshapps.GameHandler/LICENSE` is
   35,149 bytes and `cmp`-identical to the repo's `LICENSE`, alongside
   `DXVK.txt` and `osslsigncode-LICENSE.txt`. The exported desktop file is
   `~/.local/share/flatpak/app/com.goshapps.GameHandler/current/active/export/share/applications/com.goshapps.GameHandler.desktop`, and it is the repo file
   **with Flatpak's own correct rewriting**: `Exec=gamehandler` becomes
   `Exec=/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=gamehandler com.goshapps.GameHandler`
   and `X-Flatpak=com.goshapps.GameHandler` is added — a `diff` shows those two
   changes and nothing else. The exported metainfo is **byte-identical** to the
   repo's (`cmp` clean).
5. **The `base`'s files do not leak into the app's export.** The build tree's
   `/app/share` contains `org.winehq.Wine.desktop`, `winetricks.desktop`,
   `org.winehq.Wine.metainfo.xml` and `io.github.winetricks.Winetricks.metainfo.xml`
   — contributed by the `org.winehq.Wine` base and its winetricks module. I
   checked whether these are exported as part of this app, because exporting
   three desktop entries would be a real defect: they are **not**. The export
   `share/applications/` holds exactly one file (`com.goshapps.GameHandler.desktop`)
   and `share/metainfo/` exactly one (`com.goshapps.GameHandler.metainfo.xml`).
   Flatpak attributes them to the base, not to this app. No action needed.
6. **Version consistency: no mismatch found, across all six places a version
   appears.** `Cargo.toml:18` `version = "0.8.0"` (in `[workspace.package]`),
   inherited by `crates/app/Cargo.toml:4` `version.workspace = true` and
   `crates/core` the same way; `meson.build:3` `version: '0.8.0'`;
   `data/com.goshapps.GameHandler.metainfo.xml:49` `<release version="0.8.0"
   date="2026-09-11">`, the newest of 11 release entries (the next is `0.7.2`);
   `build-flatpak/files/bin/gamehandler --version` → `GameHandler 0.8.0`;
   `flatpak info com.goshapps.GameHandler` → `Version: 0.8.0`; and the bundle
   `dist/gamehandler-0.8.0.flatpak` (103,611,752 bytes), whose name comes from
   `build-aux/flatpak/build.sh:24-28,47`. The `.desktop` file carries no version
   field, which is correct — desktop entries have no such key. `build.sh`'s
   two-step derivation (member `Cargo.toml` first, workspace root as fallback) is
   deliberate and documented at `build.sh:10-13`, so it is a fixed rule rather
   than an accident; it resolves to `0.8.0` today because `crates/app/Cargo.toml`
   sets `version.workspace = true` rather than its own literal.
7. **Licensing is consistent and the licence text actually ships.** The declared
   licence agrees in all three places it is stated: `Cargo.toml:22` `license =
   "GPL-3.0-or-later"` (inherited via `license.workspace = true`),
   `data/com.goshapps.GameHandler.metainfo.xml:5` `<project_license>GPL-3.0-or-later</project_license>`,
   and `flatpak info com.goshapps.GameHandler` → `License: GPL-3.0-or-later`
   (which it reads from the metainfo, so the metainfo is what an installed app
   reports). The metadata licence is `CC0-1.0` (`:4`), which is the correct
   choice for the metadata file itself and is a different thing from the project
   licence. The GPL text is installed as a file (item 4), and the two
   third-party licences the build redistributes — DXVK and osslsigncode — are
   installed beside it.
8. **The declared installs and the built tree agree, and the completeness check
   is real.** `verify.sh`'s `FLATPAK_CONTENTS` (`:1544-1550`) names five
   non-binary installs, and the manifest declares each with the exact source and
   destination (`install -Dm0644` at `…json:91`, `:92`, `:93`, `:94`, `:95`; the
   binary at `:90` is exempted at `verify.sh:1510`). `verify.sh` does not merely
   compare the list against itself: `DECLARES_PY` (`:1475-1496`) parses the
   manifest's `build-commands` and requires the command to begin with `install `
   and the token before the destination to equal the source, and
   `COMPLETENESS_PY` (`:1558`) fails if the manifest installs something the list
   omits. Against the built tree the stage goes further and `cmp`s each installed
   file against its repo original (`:1763-1773`). I confirmed the end state
   independently (item 4). This is a well-built check; its gap is what it does
   *not* run (PKG-04), not what it asserts.
9. **The `desktop-metainfo` stage is honest about being unable to run.** It runs
   both validators on the built copies when a tree exists and the repo copies
   otherwise (`verify.sh:1375-1390`), and — importantly — distinguishes a
   validator **verdict** from a **missing validator**: a failed verdict returns
   1 (`:1397`), while an absent tool returns `99` (`:1398`), which maps to an
   unrequested skip and **exit 3** (`:1981`, `:2118-2123`). So "the validator was
   not installed" can never be reported as a pass. This is the correct handling
   of a prerequisite, and it is the pattern PKG-03 and PKG-07's missing
   `appstream-util` land in.
10. **The `banner_check` limit is documented rather than hidden.** The check
    compares the sequence of stage banners with the sequence of `STAGES` entries,
    and cannot detect a banner moved to a different function without changing its
    position. The script says so itself, at length, and caps what a green result
    means: "Treat a green `banner_check` as 'the names and their order agree',
    which is what it says, and nothing more" (`verify.sh:216-224`). A partial
    check with its limit written down next to it is not a defect, and I am not
    recording it as one — the opposite, since it is the pattern the rest of this
    audit would like to see more of.
11. **The build outputs are correctly untracked.** `build-flatpak/`,
    `flatpak-repo/`, `dist/` and `/target` are in `.gitignore` (`:13`, `:17`,
    `:18`, `:19`), and `git ls-files` returns nothing under any of them, so a
    stale build tree cannot be committed and mistaken for a source of truth. The
    tracked Flatpak inputs are exactly three files: the manifest,
    `cargo-sources.json` and `build.sh`.
12. **`smoke-test.sh`'s exit-code contract is well-designed.** `0` = every check
    that could run passed, `1` = at least one check failed, `77` = only the CLI
    checks ran because no compositor was available, implemented at
    `smoke-test.sh:444-453` and translated by `verify.sh:1364-1368`. The
    `77` case is described in the script as "Reported loudly, never green"
    (`:37-38`), and the skip path prints "NOT a pass: the GUI has not been
    exercised" (`:440`). The `cli-list-exits-0` check is explicitly shallow and
    says so in its own success message — "output not examined"
    (`smoke-test.sh:252-253`) — and the `no-display-diagnostic` check does real
    work: it rejects exit 0 (`:271-273`), rejects silence (`:274-275`), and
    matches a panic pattern (`:228`). The `gui-stays-up` check is liveness plus
    panic-scan plus SIGTERM-reap (`:396-425`), which is a weaker property than
    "rendered correctly" — but the script never claims otherwise, which is why
    this is here and not in the findings.

## Not verified

1. **Whether the exported bundle installs and runs.** `flatpak build-bundle`
   produced `dist/gamehandler-0.8.0.flatpak` (103,611,752 bytes), but I did not
   install it into a clean installation and run it, and the sandbox-execution
   gap (PKG-04) is exactly this. The test that settles it: `flatpak
   --user install --bundle dist/gamehandler-0.8.0.flatpak` on a machine with no
   prior GameHandler, then `flatpak run --command=gamehandler
   com.goshapps.GameHandler --version` and `--list`, and confirm the GUI starts
   under a compositor.
2. **Whether the tree's own build (`build-flatpak`) is the source of the
   installed app.** They are different binaries by hash (PKG-01) and the
   installed commit is dated `2026-09-12 17:56:44`, but I did not reconstruct
   which commit each was built from. Settling that needs the build log of each,
   or a rebuild-and-compare; the practical fix is to print the hash (PKG-01)
   rather than to date the artefacts.
3. **Why the binary is stripped, given that no profile says so.** `file
   build-flatpak/files/bin/gamehandler` reports `stripped`, and `readelf -S`
   finds **0** `.symtab` sections, but `grep -rn "strip\|lto\|codegen-units\|panic"`
   over `Cargo.toml`, `crates/app/Cargo.toml` and `.cargo/config.toml` returns
   **nothing** — there is no `[profile.release]` in the repository. So the strip
   comes from the build environment (the rust-stable SDK extension, or
   `CARGO_HOME=/run/build/gamehandler/cargo`), not from anything auditable here.
   Consequence: a build of the same source outside the SDK may produce a
   different-sized artefact, which interacts with PKG-01's hash comparison.
   Settling it needs the SDK's cargo config, or a build with and without
   `strip = true` and a size comparison.
4. **Whether the 99 MB bundle and the 505.9 MB reported installed size are
   expected.** `flatpak info com.goshapps.GameHandler` reports `Installed Size:
   505.9 MB`. Much of the build tree's `/app/bin` is the Wine base's tooling
   (`7zz`, `cabextract`, `cifsdd`, `dbwrap_tool`), and with a `base` the app and
   base are separate refs, so I could not attribute the installed figure between
   them from the filesystem alone. Settling it needs `flatpak info --show-size`
   style accounting, or a comparison against a build with a different base.
5. **Whether `--pedantic`'s `cid-contains-uppercase-letter` warning would fail a
   real Flathub submission.** Flathub's own linter is not the same program as
   `appstreamcli`, and I could not run Flathub's checks offline. Settling it
   needs a submission, or `flatpak-builder-lint` if it can be obtained — note
   that `appstream-util`, which the project's own `data/meson.build:50` also
   accepts, is not installed here either, so the metainfo has been checked by one
   validator, not two.
6. **The screenshots' existence and reachability.** Adding `<screenshots>` (PKG-06)
   requires images hosted over HTTPS; I did not check whether any exist for this
   project, because none are referenced. Settling it needs the hosting decision
   first, then `appstreamcli validate` without `--no-net`, which would have
   fetched them.
7. **Whether the Flatpak build succeeds right now from a clean state.** A
   `flatpak-builder` run is destructive to `build-flatpak` and the brief's
   instruction was not to interfere with a build in flight, so I audited the
   manifest, the pins and the built tree rather than re-running the build from
   scratch. `verify.sh`'s own `flatpak-build` stage holds
   `FLATPAK_LOCK="$ROOT/target/verify-flatpak.lock"` (`:627`) and uses
   `--force-clean` (`:1344`), so it is safe to run deliberately — it just was not
   run here.
