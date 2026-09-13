# Performance and resource audit

## Scope

This is a read-only audit of the runtime cost of GameHandler (`crates/core` =
`gamehandler-core`, the GUI-free half; `crates/app` = the `gamehandler` binary,
libcosmic UI over the `iced_tiny_skia` **software** renderer), performed against
the tree at commit `d56782d`. **No source file was modified.** The seven areas
examined were, in order: redraw behaviour (does anything wake an idle app, and
what does one redraw cost on a CPU renderer); startup cost (the project's claim
that startup performs exactly two filesystem reads); per-frame allocation inside
`view()`; image and cover handling; search and filter; unbounded growth; and
resource leaks.

Every claim cites a `file:line` I read or a command I ran, with its output. Where
a number is a measurement, the method is given with it. Where something could not
be settled, it is in **Not verified** with the test that would settle it.

**Measurement environment, and its caveats.** All runtime numbers come from the
binary built by `flatpak-builder` in this tree
(`build-flatpak/files/bin/gamehandler`, 29,458,152 bytes, `GameHandler 0.8.0`)
executed **outside** the sandbox, with `XDG_CONFIG_HOME`/`XDG_DATA_HOME`/
`XDG_CACHE_HOME`/`XDG_RUNTIME_DIR` pointed at a private tree under `/tmp/gh-perf`
and a library fixture of 500 games (`/tmp/gh-perf/config/gamehandler/games.json`,
127,823 bytes) or an empty one. Redraws were forced by changing the output
resolution on a private headless `sway`; see *Verified sound* item 8 for why that
is a valid stimulus. **The machine was not quiet: `/proc/loadavg` read
`5.03 10.39 9.71` during measurement.** Absolute CPU figures are therefore
inflated relative to an idle machine, and the run-to-run spread I observed
(5.2–24.7 ms of CPU per redraw stimulus across all runs) is at least partly the
host, not the app. The *ratios* between configurations were stable and are what
the findings rest on.

**Headline results.** Two properties the brief asks about are confirmed sound:
**there is no idle spin** — an idle app consumes 0 CPU ticks and issues **zero**
filesystem syscalls over tens of seconds, with all 27 threads blocked — and
**covers are decoded once and cached across frames, not re-decoded per frame**.
But the per-frame path is far from free, and three findings are serious on a
software renderer: every game's cover is `stat`ed *and* `open`ed on **every**
frame whether or not it is visible (PERF-01); decoded covers are retained at full
source resolution, which I measured taking a 500-game library to **753 MB** of
RSS (PERF-02); and nothing is virtualized, so all N tiles are built, filtered,
sorted and laid out every frame (PERF-03).

## Findings

Sorted most severe first.

| ID | Severity | Finding | Evidence (`file:line` or command + output) | Impact | Recommendation |
|---|---|---|---|---|---|
| PERF-01 | P1 | **Every game's cover is classified on every frame — two filesystem syscalls per game with a cover, per frame — regardless of whether the tile is visible.** `CoverSource::classify` does `Path::new(cover_path).is_file()` (a `statx`) and then, when the file exists, `has_ico_magic` → `File::open` + `read_exact(&mut magic)` (an `openat` + `read`). It is called from `cover_plan` and again from `framed`, i.e. the work is done twice per rendered cover. Nothing culls it: the cull in the renderer (`if !_clip_bounds.intersects(&physical_bounds) { return; }`) happens at *draw* time, long after `view()` has run. | `crates/app/src/view/cover.rs:112-123` (`classify`, the `is_file()` early return at `:115`) and `:192-198` (`has_ico_magic`); `crates/app/src/view/widgets.rs:254-255` (`cover_plan`) and `:727-728` (`framed`); the cull is `iced/tiny_skia/src/engine.rs:585-588`. **Measured**, `strace -f -tt -e trace=%file` over a 3.572 s window of 100 forced redraws with 333 covers on disk: **6,624 cover-file syscalls (2,664 `statx` + 3,960 `openat`), and all 333 distinct cover paths were touched** — every cover, every frame, in a 1200×800 window that shows a few dozen tiles. With the cover files absent (the `Plate` path returns before the open) the same harness measured **9,990 `statx`, exactly 333 per frame across 30 frames**. | This is the dominant per-frame I/O cost and it scales linearly with library size, not with what is on screen. A 2,000-game library pays ~2,000 `statx` + ~2,000 `openat` per frame — on a software renderer that is already spending ~5–25 ms of CPU per redraw, so a large library spends a meaningful fraction of every frame in the kernel resolving files whose answer cannot have changed since the last frame. This is the same defect class the project already documents for the runner label (see PERF-06): the value is a pure function of data that only changes when the library is re-read. | Resolve the cover kind **once**, where the rows are built, and carry it as data — exactly the fix `crates/app/src/view/library.rs:253-271` already prescribes for the runner label ("a `State` field holding the resolved rows, updated where the library is re-read"). `CoverPlan` is already a value type with its own tests (`widgets.rs:240-259`), so it can be computed at load and stored alongside the game with no change to what is drawn. Failing that, at minimum compute it once per rendered cover per frame instead of twice. |
| PERF-02 | P1 | **Cover images are decoded at full source resolution and retained, and the cache is bounded only by "what was drawn in the last frame" — which, with no virtualization (PERF-03), is every game in the library.** The decoded pixmap is stored as `vec![0u32; image.width() * image.height()]`, i.e. 4 bytes per pixel at the *source image's own* dimensions, with no downscale-on-load; a separately resampled copy is cached under `(id, target_w, target_h)`, also at 4 bytes per pixel, and **both** are kept. The per-frame trim retains only entries hit in that frame, so nothing is evicted while it is still being drawn. | Cache and allocation: `iced/tiny_skia/src/raster.rs:123-128` (`entries: FxHashMap<raster::Id, Option<Entry>>` + `resampled`), `:151-171` (the native-resolution `vec![0u32; w*h]`), `:185-219` (`allocate_resampled`, a second full-size buffer), `:232-237` (`trim` retains only `hits`); the trim runs once per frame at `iced/tiny_skia/src/lib.rs:198`. **Measured**, RSS of the 500-game library read from `/proc/<pid>/status` `VmRSS` after the window had drawn, covers written as real JPEGs at each size: **19,860 kB with no covers** (figure supplied by the coordinator's measurement of the sandboxed app; my own no-cover runs were in the same ~20 MB band); **144,564 kB with 333 covers at 200×300**; **771,656 kB with 333 covers at 600×900**. The last is the decisive one: 333 × 600 × 900 × 4 bytes = **719 MB**, against an observed increase of 753 MB − 20 MB = **733 MB** — so essentially every one of the 333 covers is decoded and resident, not merely the few dozen the window can show. | Memory grows as `(native pixels × games)` with no cap. The 600×900 figures are not a stress test: 600×900 is an ordinary cover, and 753 MB of RSS for a 500-game library is enough to get the app OOM-killed on a small machine or a Steam Deck. A library of 500 games with 1000×1500 covers would be ~3 GB, and the app holds **both** the native and the resampled copy. | Downscale on load to the largest size the UI can draw (the grid cell is 200 px wide per `LibraryPage.qml:139`, cited at `crates/app/src/view/library.rs:433`) and store only that, so `entries` never holds a full-resolution pixmap; and/or bound the cache by total bytes rather than by "hit last frame", since the last-frame rule is exactly what makes residency equal to library size here. Any fix should be measured with the method above — real cover files at a known size, `VmRSS` after a draw. |
| PERF-03 | P1 | **Nothing is virtualized: every game in the filtered library gets a built `Element` every frame.** `list_body` loops over all `shown` games pushing a `context_menu(row(...))` per game, and `grid_body` does the same with `card(...)`; `view()` is called on every frame and there is no `lazy`, `Responsive`, or any viewport-limited child. The only bound on the work is `search()`, which does not reduce the count when no filter is set. | `crates/app/src/view/library.rs:377-397` (`list_body`, the `for game in games` loop at `:379`, the per-row push at `:393-397`), `:436-456` (`grid_body`, loop at `:440`, per-card push at `:443-447`), reached unconditionally from `:358-363`; no `lazy`/`responsive` anywhere in the file (`grep -n "lazy\|Lazy\|responsive" crates/app/src/view/library.rs` → no matches). The decoded-image cull at `iced/tiny_skia/src/engine.rs:585-588` confirms the renderer, not the view, is where off-screen work is dropped. | View construction, layout and diffing are O(library size) **per frame**, on the UI thread, on a CPU renderer. This is also the multiplier that makes PERF-01 and PERF-02 apply to the whole library rather than to the visible page, and it is why the cover cache has no natural bound. | Wrap the body in iced's `lazy` (or a virtualized list) so only the visible rows plus a small overscan are constructed, and drive it from a `(library revision, query, category, sort, scroll offset)` key. That single change removes the O(N)-per-frame term from view construction, from the cover `stat`/`open` (PERF-01), and from the decoded-cover residency set (PERF-02). |
| PERF-04 | P2 | **The project's claim that startup performs only two filesystem reads is false, and the comment asserting it sits three lines above the calls that contradict it.** `App::init` runs `Settings::load(None)` and `Library::new(None)`, immediately followed by `state.refresh_plugins(&SystemPluginEnv)` and `state.refresh_installers()`. `refresh_plugins` walks `PATH` once per plugin for five plugins and probes `/.flatpak-info`; `refresh_installers` runs `installer_categories()`, and `runner_choices()` → `installed_protons()`, a `read_dir` over the runners directory plus a metadata read per runner. | The claim: `crates/app/src/main.rs:3653-3655` ("The two loads are the only filesystem work done at startup, and neither is fallible"), with the contradicting calls at `:3656-3668`. The work: `crates/app/src/state.rs:916-919` (`refresh_plugins`), `:948-960` (`refresh_installers`); `crates/core/src/plugins.rs:84-86` (`is_installed` → `env.which`), `:258-278` (`detect_package_manager`, up to five `which` scans plus two `/etc` probes), `:437-500` (`plugin_row`/`plugin_rows`, which reach `detect_package_manager` and `in_flatpak`), `:197-240` (`SystemPluginEnv`: `which_in` at `:207`, `Path::exists` at `:210`, `effective_uid()` reading `/proc/self/status` at `:233-238`). **Measured**: `strace -f -tt -e trace=%file` shows **65,279 file syscalls in the first second** and 2,292 in the second, including **17 `statx` of `/.flatpak-info`** — a path only `in_flatpak()` opens, and `in_flatpak` is reached only from `plugin_rows`/`plugins_intro`/`detect_package_manager`, i.e. only from `refresh_plugins`. By contrast the CLI path is genuinely minimal: `strace -f -c -e trace=%file` on `--list` with the 500-game library shows **18 file syscalls total** (15 `openat`, 1 `statx`, 1 `access`, 1 `execve`). | The direct cost is real but modest (a PATH scan and a handful of probes, negligible beside the ~62,500 icon-theme lookups libcosmic itself performs at startup in my run). The durable cost is that a comment stating an invariant that the code does not hold will defeat the next person who measures startup cost, and this project's own audit history shows that class of defect recurring. | Correct the comment, or make it true by moving `refresh_plugins`/`refresh_installers` onto the existing post-startup `Task` path so the window can paint first. Note the icon-theme scan (62,510 of the 67,350 path-bearing startup syscalls, all under `/usr/share/icons` and the host's Nix store) is libcosmic's, not this app's, and is far smaller inside the Flatpak — worth not attributing to this code in any future write-up. |
| PERF-05 | P2 | **`search()` re-sorts and re-filters the entire library, and `categories()` rebuilds and re-sorts the category list, on every frame — and both allocate `String`s inside sort comparators.** `view()` calls `page.library.categories()` and `page.library.search(...)` unconditionally, on every frame, not only when a query changes. `search` calls `all(sort)`, which clones the game list and sorts it; the comparators call `a.name.to_lowercase().cmp(&b.name.to_lowercase())`, allocating two `String`s per comparison. `categories()` collects an owned `String` per game and sorts with `to_lowercase()` twice per comparison. | `crates/app/src/view/library.rs:286-291` (both calls, at the top of `view()`, `:286-291`); `crates/core/src/models.rs:625-639` (`all`; the comparator allocations at `:631`, `:636` and the `sort_by_key(|a| a.name.to_lowercase())` at `:638`); `:684-698` (`search`: `query.trim().to_lowercase()` at `:685`, `self.all(sort)` at `:686`, the two `retain` passes at `:688-698`); `:702-716` (`categories`: `map(...to_owned())` at `:706` and the `to_lowercase` comparator at `:711`). | For an N-game library this is an O(N log N) sort with O(N log N) heap allocations per frame, plus a full `Vec<String>` rebuild, all to produce a value that changes only when the library, the query, the category or the sort key changes — none of which change during a redraw. The `to_lowercase()`-in-comparator pattern multiplies the cost by roughly the comparison count and is trivially avoidable (sort by a precomputed key, or use `eq_ignore_ascii_case`). | Memoize the resolved rows on `(library revision, query, category, sort)` — the same `State` field `crates/app/src/view/library.rs:253-271` already proposes — and recompute only when one of those inputs changes. Replace the comparator `to_lowercase()` calls with a precomputed lowercase key or a case-insensitive comparison. |
| PERF-06 | P2 | **`RunnerManager::label` is uncached, walks the filesystem, and is called once per shown game per frame — and it reads and JSON-parses each runner's metadata only to discard the parsed value.** The code documents this against itself, and the fix, at length. `label()` checks `candidate.exists()` and then constructs a `ProtonRunner` via `discovered()`, whose `new()` eagerly calls `read_family_id` → `read_metadata`, which reads and `serde_json`-parses the runner's `.gamehandler.json`; only `family_label()` and `name()` are used, and the parsed `Value` is dropped. | The self-documenting comment: `crates/app/src/view/library.rs:253-271` ("**This is called from the render path, and that is a known cost** … `RunnerManager::label` is uncached and walks the filesystem. `view` runs every frame, so taking the manager here reintroduces exactly that — once per shown game per frame."). The implementation: `crates/core/src/runners/mod.rs:1093-1110` (`label`), `:621-644` (`ProtonRunner::new`, the eager `read_family_id` at `:626-630`), `:971-989` (`read_metadata`: `root.join(METADATA_NAME)` at `:975`, `meta.exists()`, `std::fs::read`, `String::from_utf8`, `serde_json::from_str`), `:992-1000` (`read_family_id`). Call sites: `crates/app/src/view/library.rs:383` (`row_labels` → `resolved_runner_label`, inside the per-game loop) and `:441` (`grid_body`). **Measured**: 3,750 `statx` on `/tmp/gh-perf/data/gamehandler/runners` in the 3.572 s / 100-redraw window (`strace`, same harness as PERF-01). | Each frame performs a `statx` plus a file read plus a JSON parse per row, for a value that changes only when a runner is installed or removed. Note also that `ProtonRunner::new`'s eager metadata read is a regression against the reference's *lazy* `@property` — `read_family_id` is paid even when the caller only wants `name()`, which is exactly what `label()` does. | Resolve the label when the library is re-read and store it in `State` (the fix the comment names); at minimum make `family_id` lazy as the Python was, so a caller that only needs the display name does not read and parse metadata. |
| PERF-07 | P3 | **The 27 threads are all demand-spawned runtime workers; none polls, and two of them exit on their own.** The question of whether the thread count hides an always-running poller is settled by reading each thread's blocking state. Every one of the 27 was parked in a blocking wait: 3 `gamehandler` (main), 3 `notify-rs inoti` (the filesystem watcher), 1 `smithay-clipboa`, and 1 `tokio-rt-worker` in `do_epoll_wait`; the remaining 19 `tokio-rt-worker` threads in `futex_do_wait`. Two threads exited spontaneously at t+10.03 s, which a fixed always-running pool would not do. | Method: `/proc/<pid>/task/*/stat` field 3 (name) and `/proc/<pid>/task/*/wchan` inspected for every tid of the running app; thread count from `/proc/<pid>/status` `Threads`. Confirmed stable: `Threads` read 27 at t=10 s, 20 s, 30 s, 40 s, 50 s, 60 s and 70 s in one 70 s run, and 27 in every other run. The two exits appear in the timestamped `strace -f` as `+++ exited with 0 +++` at t+10.032 s and t+10.034 s. | None: this is the expected shape of a tokio-based libcosmic app, and it is recorded because "27 threads on an idle app" invites the opposite reading. | No change. If the thread count is ever revisited, the two that exit are the ones to account for, not the 25 that persist. |
| PERF-08 | P3 | **Minor RSS drift that is not attributable to a leak on the evidence gathered.** Across a 60 s run with six forced redraws at six *different* window heights, RSS rose from 771,420 kB to 772,492 kB (+1,072 kB) while fd count and thread count were exactly constant. I could not attribute this to any single cache. | `perf-leak.sh` sampling loop; output `t=10s fds=42 threads=27 rss=771420kB` … `t=70s fds=42 threads=27 rss=772492kB`. | If it is the raster cache, the growth is explained by six distinct window sizes each producing a new `(id, width, height)` resampled entry (`iced/tiny_skia/src/raster.rs:192`) — but at ~179 kB per redraw this is not material at any realistic rate, and the same run shows no fd or thread growth at all. | No action needed on this evidence. If a long-session leak is suspected, sample `VmRSS` over hours of *idle* time rather than over forced resizes, since the resize itself changes the cache key set. |

## Verified sound

1. **No idle spin. This is the headline negative result and it is settled three
   independent ways.** (a) My measurement: `/proc/<pid>/stat` fields 14+15
   (utime+stime) sampled before and after a wall-clock sleep at `CLK_TCK`=100 —
   PID 3683396, 500-game library, private headless weston: **0 ticks over
   15.0042 s = 0.00 % of one core**; repeated under sway, PID 3698775, window
   `1200x773`: **0 ticks / 15.00 s = 0.00 %**. (b) The coordinator's independent
   measurement of the **sandboxed** app — and note the process tree matters,
   because `/tmp/gh-walk/app.pid` holds the outer `bwrap` wrapper (1 thread,
   2.2 MB) and measuring *that* would have told us nothing; the real app is its
   child: **0 ticks over 15 s at `CLK_TCK`=100**, RSS 19,860 kB, 27 threads.
   (c) Stronger still, my timestamped `strace -f -tt -e trace=%file` run shows
   **zero filesystem syscalls of any kind in 22 s of idle** — after the startup
   burst, the only events in the trace are two thread exits at t+10.03 s and the
   SIGTERM teardown. An app that polled a directory, a socket or a timer would
   not be silent for 22 s. My own later runs reproduce it with covers present:
   `idle_ticks_over_8s=0` at both cover sizes.
2. **The code corroborates (1).** The app's only subscription is
   `cosmic::iced::keyboard::listen().filter_map(...)`
   (`crates/app/src/main.rs:3747`) — purely event-driven. libcosmic's default is
   `Subscription::none()` (`libcosmic src/app/mod.rs:460-462`), and everything
   `Cosmic::subscription()` adds is event-driven: `watch_config`, `window_events`,
   `theme::portal::desktop_settings()` (which sends only on a real
   colour-scheme/contrast change and backs off exponentially on failure,
   `src/theme/portal.rs:15-55`), and the Wayland `text_context_menu`
   `wake_subscription`, which is a `while rx.next().await.is_some()` loop over a
   channel, not a timer (`src/widget/text_context_menu.rs:157-185`). There is no
   `time::every` anywhere in the app. `Message::LaunchWatchTick` exists but is a
   deliberately empty arm that **no production code constructs**
   (`crates/app/src/main.rs:2580`), so it cannot wake anything.
3. **Covers are decoded once and cached across frames — not re-decoded per
   frame.** This was the specific risk the coordinator asked about and it does
   *not* occur. `image::Handle::from_path` is path-keyed and performs no decode
   (`iced/core/src/image.rs:125-129`: `Self::Path(Id::path(&path), path)`), and
   the renderer's raster cache keys the decoded pixmap on that id
   (`iced/tiny_skia/src/raster.rs:123-128`, `:138-171`), so a repeated frame is a
   cache hit. The cost that *does* recur per frame is the classify in PERF-01 and
   the residency in PERF-02 — different problems with different fixes.
4. **`view()` does not clone the whole library, nor per-game cover data.** The
   clones in the render path are small and per-item: `Message::LaunchGame(game.id.clone())`
   (`crates/app/src/view/library.rs:394`, `:444`) and the category/option strings
   captured into dropdown closures. `search()` returns `Vec<&Game>` — borrowed,
   not cloned. Nothing in `view()` deep-copies a `Game` or an image buffer. The
   per-frame cost is real but it is **work**, not cloning.
5. **Toasts are bounded and do not leak timers.** `Toasts` holds at most 5
   (`libcosmic src/widget/toaster/mod.rs:164`, evicted in `push` at `:174-179`),
   and each `push` schedules a single one-shot `tokio::time::sleep`
   (`:186-193`) — one task per toast, not a repeating timer.
6. **Blocking launch work does not run on the UI thread.** `wait_with_timeout`
   polls `try_wait()` in a loop with `std::thread::sleep(10ms)`
   (`crates/core/src/runners/launch.rs:279-301`), which would freeze the UI if it
   ran in `update()`. It does not: `Message::LaunchGame` spawns a detached
   `std::thread` and streams messages back over an unbounded channel
   (`crates/app/src/main.rs:2052-2063`), and `Task::stream` delivers them. The
   thread is detached rather than joined, which is correct here — it ends when
   the launch completes and the receiver drops.
7. **No fd, thread, child-process or file-growth leak over 70 s.** One run
   sampled every 10 s: `fds=42` at every one of seven samples, `threads=27` at
   every sample, RSS effectively flat (see PERF-08). `ps --ppid <pid>` reported
   **0** unreaped children; the app wrote **0** new files under `XDG_CACHE_HOME`
   or `XDG_DATA_HOME`; and its stdout+stderr were **0 bytes** — there is no log
   file, so "unbounded log growth" does not apply. The one runtime persistence
   path is atomic by construction: `write_python_file` writes a `json.tmp`
   sibling then `rename`s it into place (`crates/core/src/json.rs:356-365`), so a
   crash cannot truncate a library.
8. **The redraw stimulus is valid, and the frames are healthy.** A private
   headless `sway` (`WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1
   WLR_RENDERER=pixman`) with `swaymsg output <name> resolution WxH` forces a
   genuine resize and repaint — a stronger and more reproducible cost stimulus
   than a synthetic click, and the only option available, since the default
   weston headless backend exposes no virtual-pointer protocol. That the app is
   really drawing is confirmed by capture, not assumption: `grim` produced a
   58,527-byte PNG of the 1200×800 window showing the Library page fully
   rendered — sidebar (Library / Installers / Runners / Plugins / Credits /
   Settings), search box, category "All", Sort "Name", the grid/list toggle, a
   "500 games" header and a populated scrollbar. The per-redraw figures below are
   therefore measured against real frames, not a blank or stalled one.
9. **Redraw CPU cost, for the record (same harness, 100 resolution changes per
   run, ticks from `/proc/<pid>/stat`):** 166 ticks (1.66 s of CPU) with an
   **empty** library; 216 ticks with 500 games and covers absent; 247 ticks with
   500 games and 333 covers present; and a separate pair of earlier runs at 5.40
   and 11.92 ms of CPU per redraw. Across all runs the range is **~5.2–24.7 ms of
   CPU per redraw stimulus**, with the spread attributable to the loaded host.
   The one robust signal is that a 500-game library costs measurably more CPU
   per redraw than an empty one (+50 ticks, ~30 %) — consistent with PERF-01,
   PERF-03 and PERF-05 being real per-frame work that scales with library size.

## Not verified

1. **The exact number of frames rendered per redraw stimulus.** My figures are
   per *stimulus*, not per frame. The clean case (`Plate` path) gives exactly
   9,990 cover `statx` across 30 clusters of 333, which is strong evidence for 30
   frames; the `Photo` path's syscalls do not cluster as cleanly (the decodes
   interleave), and the fit that works (10 frames × 333 covers × 2 syscalls =
   6,660 ≈ the 6,624 measured) is an inference, not a measurement. Settling it
   needs a frame counter — a `log` line per `view()` call, or instrumenting the
   renderer — which I did not add, because the brief forbids modifying source.
2. **Per-frame cost inside the real Flatpak sandbox.** All measurements used the
   `flatpak-builder` tree binary run outside the sandbox with a private
   `XDG_*` tree. Inside the sandbox the icon-theme scan should be far smaller
   (62,510 of the startup syscalls were host icon lookups, and `/.flatpak-info`
   short-circuits `detect_package_manager`), so the *startup* figure in PERF-04
   is an upper bound. The per-frame findings (PERF-01/02/03/05/06) should be
   unaffected, since they are app code — but I did not run them in-sandbox.
3. **The per-thread role of the 27 threads beyond their blocking state.** I read
   each thread's name and `wchan`, which is enough to show none is running, but
   not enough to say which subsystem owns each `tokio-rt-worker` or which two
   exited at t+10 s. Settling it needs a profiler or `RUST_LOG` instrumentation.
4. **Real-world cover sizes and the resulting memory in practice.** PERF-02's
   covers were generated with `ffmpeg` at exactly 200×300 and 600×900. Real
   libraries contain whatever the user chose (screenshots, scans, 4K art), so the
   753 MB figure is a demonstration of the *scaling law* — 4 bytes per pixel per
   game, native resolution, both native and resampled copies — not a prediction
   for a particular library. The law itself is verified; the population
   distribution is not.
5. **The cause of the PERF-08 RSS drift.** Six distinct window sizes produce six
   distinct resampled cache keys, which is a sufficient explanation for +1,072 kB
   but not a verified one. Settling it needs a run where the window size is held
   constant, or a byte-accounting of the raster cache.
6. **Behaviour under the `wgpu` feature or on a GPU.** This audit is of the
   `iced_tiny_skia` software renderer only, which is what the build selects
   (`crates/app/Cargo.toml`, `wgpu` deliberately absent). The numbers here should
   not be transferred to a GPU-backed build, where the cover-decode cache
   (`iced/wgpu/src/text.rs`, atlas-based) has a different residency model.
7. **Whether any of this is visible to a user on an unloaded machine.** With
   `/proc/loadavg` at 5.03 during measurement, the absolute CPU figures are
   inflated. A quiet-machine rerun of the same harness would give a fairer
   baseline for the "is 20 ms per redraw a jank problem?" question, which is a
   UX judgement I have the cost for but not the feel.
