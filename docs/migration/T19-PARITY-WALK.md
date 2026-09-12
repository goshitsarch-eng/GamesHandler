# T-19 — the independent parity walk (P-01…P-78)

**Owner:** Advocate (T-19). **Standard:** `docs/migration/review-phase1.md` §2.
**Measured against:** `9bcacf9` in the isolated sandbox `/tmp/t19`
(`git archive 9bcacf9 | tar -x`, `CARGO_TARGET_DIR=/tmp/t19-target`), with
cross-checks against the live working tree at `ae90dab` where a teammate's
in-flight change matters. Every claim below is labelled with the tree it was
read from.

**Why this document exists.** T-19's exit criterion in `PLAN.md` is a
verification pass, and §2's Verify column is written as *prose about the Python
app*: every citation in it (`test_library_view.py:41`,
`test_security.py:97-190`, `test_installers.py:487-560`, `test_credits.py`,
`test_covers.py:65-83`) names a test in `tests/`, and **every file in `tests/`
imports `gamehandler.*` — the Python reference — not `crates/`.** Only
`tests/test_plan_traceability.py` reads the Rust tree, and it covers `T-nn` and
not `P-nn`. So §2's Verify column cannot be discharged by running `pytest`; it
has to be re-derived against the port. That re-derivation is this document.

**The rule applied to every row.** A test's *name* is not evidence of what it
checks (#38, #39, #46, #50, #101). Each (A) entry below names the assertion, not
the test, and says what the assertion does **not** cover. Where I could not read
the assertion, the row is not (A).

## Buckets

- **(A) — verifiable now from source and/or the existing test suite.**
  52 items. The entry names `file:line` and what it covers; a residual that the
  named evidence does not reach is stated in the entry and is usually
  *reachability*, not correctness.
- **(B) — needs the running app or GUI.** 4 items. The entry is an executable
  procedure.
- **(C) — unmet, or implemented and unreachable.** 22 items. The entry is the
  defect.

**Counts: 52 (A) / 4 (B) / 22 (C) = 78.**

The distribution is not a surprise once the shape of the port is visible:
`crates/core` is very deeply tested — `archive.rs`, `launch_opts.rs`,
`installers.rs`, `netpaths.rs`, `models.rs`, `json.rs` and `plugins.rs` all carry
assertion-level tests written against the reference's own source. Almost every
(C) is at the **UI-reachability layer**: an arm that exists in `Shell::update`
with no constructor anywhere in `crates/app/src/view/`, or a state field no
reader consults. The two source-reading guards that do exist
(`crates/app/tests/dispatch_coverage.rs`, `crates/app/tests/pending_pages.rs`)
police the *opposite* direction — "is this handled?" — so a correct, tested,
well-commented arm that no button emits is invisible to both. That is the single
mechanism behind most of the (C) list, and it is worth saying once rather than
in each of the twenty-two entries.

---

## 2.1 Library page

### P-01 — Grid view of games with cover tiles — **(A)**

Evidence: `crates/app/src/view/metrics.rs:40` —
`pub const GRID_CELL: (f32, f32) = (200.0, 300.0);`, with the module doc
recording that the 2:3 ratio is the ratio the store art is authored for
(`covers.py:35-43`). The card widget is `view/widgets.rs::card`;
`crates/app/src/main.rs:5371` (`the_library_page_draws_the_library_and_not_the_placeholder`)
drives a real `Shell` and asserts the drawn strings come from the library.
**Not covered:** nothing asserts a *laid-out* tile is 200×300 (the constant is
the size's only authority), and no test builds three games with real cover files
and reads a grid of three. Both residuals are visual; a window would settle
them, but the constant is what the layout is computed from.

### P-02 — List view with thumbnail + subtitle + last-played — **(A)**

Evidence: `crates/app/src/view/meta.rs:165-205` —
`a_categorised_game_shows_the_category_then_the_runner`,
`an_uncategorised_game_shows_the_runner_alone`,
`a_blank_category_shows_the_runner_alone`,
`a_padded_category_is_trimmed_but_kept` — and
`crates/app/src/view/library.rs:485` (`a_rows_labels_carry_the_games_own_timestamp`),
`:521` (`an_unplayed_games_labels_say_never_played`).
**Not covered:** the row's *thumbnail* is not asserted; the labels are.

### P-03 — Grid/list toggle persisted — **(C)**

The toggle itself is met: `view/library.rs:665`
(`the_view_toggle_switches_to_the_other_mode`) and the `SetViewMode` arm.
**Persistence is not implemented at all.** `Settings::save`
(`crates/core/src/settings.rs:178`) and `save_to` (`:183`) have **no call site
in `crates/app`**: a grep for `\.save()`, `save_to`, `write_python_file` and
`fs::write` across `crates/app/src/` and `crates/app/tests/`, plus a
workspace-wide grep for callers of `Settings::save`/`save_to`, returns only
`Library::save` and core's own unit tests. `App::init` reads
(`Settings::load(None)`, `main.rs:2993`) and every mutation arm writes the
in-memory field and returns `()`. There is no `Drop` and no quit-time save:
`Message::Quit` (`main.rs:1495`) only closes the window. The reference calls
`self._save_settings()` after each of these (`bridge.py:203, 214, 227, 238, 250,
269`). Toggling to List and restarting gives Grid. The same defect empties
P-66's second half.

### P-04 — Search matches name *or* category, case-insensitive — **(A)**

Evidence: `crates/core/src/models.rs:1130` —
`search_matches_the_name_or_the_display_category`. This is the port of
`models.py:188-199` and is a direct assertion, not a name.

### P-05 — Category filter dropdown ("All" + categories) — **(A)**

Evidence: `crates/app/src/view/library.rs:576`
(`the_category_filter_puts_all_first_and_changes_nothing_else`) and
`:589` (`the_category_selector_finds_its_row_or_none`); the ordering itself in
`crates/core/src/models.rs:1162`
(`categories_sort_uncategorized_last_then_alphabetically`).

### P-06 — Sort: Name / Recently played / Recently added — **(A)**

Evidence: `crates/core/src/models.rs:1176`
(`sorting_matches_python_including_the_name_tie_break`) and `:1196`
(`timestamp_sorts_break_ties_by_name`); the selector's keys in
`crates/app/src/view/library.rs:466`
(`sort_keys_are_the_ones_the_settings_accept`).

### P-07 — Blank category folds into "Uncategorized", sorted last — **(A)**

Evidence: `crates/core/src/models.rs:949`
(`display_category_folds_blanks_but_keeps_the_stored_value`) — note the second
clause of that name, which is the interesting half: the *stored* value is kept
while the *displayed* one folds.

### P-08 — Empty-library placeholder with 3 onboarding buttons — **(A)**

Evidence: `crates/app/src/view/library.rs:566`
(`the_empty_library_offers_the_reference_three_ways_out`) and `:554`
(`the_two_empty_states_say_different_things`).

### P-09 — No-results placeholder with Clear filters — **(A)**

Evidence: `crates/app/src/view/library.rs:538`
(`a_filter_that_matches_nothing_is_not_an_empty_library`), `:676`
(`clearing_the_filters_is_a_single_message`), and `main.rs:5401`
(`a_search_that_matches_nothing_is_not_an_empty_library`).
**Not covered:** that the "Clear filters" control is *pressed* — the test
asserts the drawn strings and that one message clears both records.

### P-10 — Double-click (grid) / double-click row (list) plays — **(A)**

Evidence: `crates/app/src/view/widgets.rs:1160`
(`a_double_click_on_either_delegate_publishes_the_launch_once`). This is the
strongest kind of test in the tree and worth naming precisely: it synthesises a
real double-click through `Widget::update` at the card's centre and at the row's
centre, and asserts the **exact vector** `["launch"]` — the test's own doc says
why `contains` would be wrong (`[Launch, Launch]` satisfies "it published a
launch" and is the defect the reference's `on_press`+`on_double_click` mistake
would produce). **Not covered:** that `Message::LaunchGame` then starts a
process — that is `launch.rs`'s territory and is (A) under P-40/P-47.

### P-11 — Right-click context menu on card and row — **(C)**

**No context menu exists in the port.** `grep -rn "context_menu\|ContextMenu\|right_click\|secondary" crates/app/src/` returns nothing; `view/widgets.rs` builds a `mouse_area` with `on_double_click` only (`:426`, `:488`) and `view/widgets.rs:54` records that all card actions are funnelled into a single `Message::LaunchGame` parameter. The reference has a 9-item menu (`LibraryPage.qml:298-343`).

### P-12 — Play / Edit / Find cover art menu items — **(C)**

Same cause as P-11: no menu to hang them on. Separately, `Find cover` would not
work even if it existed — see P-24.

### P-13 — Winecfg / Winetricks / Open prefix folder — **(C)**

**Implemented in core, unreachable from the UI.** `crates/core/src/runners/launch.rs:1105`
(`winecfg_uses_the_games_own_wine_and_prefix`), `:1188`
(`winetricks_is_taken_from_path_and_its_absence_is_the_documented_error`), `:1204`
(`an_unknown_tool_is_refused_by_name`) and `:1231`
(`an_empty_prefix_path_falls_back_to_the_prefixes_directory_for_the_game_id`)
settle the core half, and `main.rs:1744` has an `OpenPrefixFolder` arm calling
`open_prefix_folder` (`:2225`). But **`Message::OpenPrefixFolder`,
`Message::RunPrefixTool` and `Message::PrefixToolRun` have no constructor
anywhere in `crates/app/src/view/`** — a grep over the view modules for
`Launch|RunPrefixTool|OpenPrefixFolder` returns only `Message::LaunchGame` in
`library.rs`. The disabled-for-Linux-games half is therefore also unobservable:
nothing reads `PrefixTool` (`state.rs:134-145`) from a drawn control.

### P-14 — Create desktop shortcut — **(C)**

**Same shape.** The writer is real and tested —
`crates/core/src/runners/desktop.rs:110` (`escape_desktop_value`), `:126`
(`desktop_exec`), `:192` (`create_desktop_shortcut`), `:252` (`| 0o111`) — and
`main.rs:1772` handles `CreateDesktopShortcut` with a toast on both outcomes
(`:1789`). But `grep -rn "CreateDesktopShortcut" crates/ --include=*.rs` returns
only `main.rs`: the declaration `:690`, the arm `:1772`, a doc reference, and
the two test samples (`:4143`, `:4599`). **No view constructs it.** This is
finding #89, reproduced.

### P-15 — Remove with confirm dialog; prefix + files left on disk — **(C)**

**The arms are empty and nothing emits them.**

```rust
// crates/app/src/main.rs
Message::ConfirmDeleteGame(_game_id) => {}
Message::DeleteGameConfirmed(_game_id) => {}
```

(`:1439` and `:1441`, the latter immediately after a `TODO(T-09) / T-10: the
confirmation overlay` comment.) `state.confirm_delete` (`state.rs:770`) is
written only by a test (`main.rs:4314`) and cleared by three arms. The core
mutation exists — `crates/core/src/models.rs:656` calls `self.games.remove(position)`
— but that call site is core's own test; `Library::remove` has no caller in
`crates/app`. So the item is unmet in every clause: no menu entry, no dialog, no
delete, and therefore no evidence about whether the prefix survives.

### P-16 — "Last played" human labels incl. future-timestamp clamp — **(A)**

Evidence: `crates/core/src/models.rs:976`
(`every_format_last_played_branch_matches_python`) and `:1011`
(`a_future_timestamp_clamps_to_just_now`). The reference's own
`test_library_view.py:161` is superseded by these.

### P-17 — Runner label per row ("Linux native" or "Name · Family") — **(A)**

Evidence: `crates/app/src/view/meta.rs:204`
(`a_linux_game_is_labelled_native_and_the_manager_is_not_consulted`) — the
second clause is the one that matters and is the reference's own subtlety.

---

## 2.2 Add/Edit game form

### P-18 — Add (Ctrl+N) and Edit paths, Add/Save gated on non-blank name — **(A)**

Evidence: `crates/app/src/view/form.rs:2108`
(`saving_is_gated_on_a_trimmed_name`), `:2195`
(`the_title_and_the_action_follow_is_new`); the drawn form in
`main.rs:5886` (`the_edit_form_draws_its_own_title_and_action`); the accelerator
in `crates/app/src/shortcuts.rs:181`
(`each_own_accelerator_produces_the_reference_s_own_message`).
**Not covered:** the reference's exact refusal *string* ("A game needs a name")
is asserted at the `GameForm::apply` boundary, not as a toast on a drawn page.

### P-19 — Windows vs Linux-native switch; Linux disables runner/prefix/Wine — **(A)**

Evidence: `crates/app/src/view/form.rs:1298`
(`a_windows_only_row_is_disabled_only_on_a_linux_game`), `:1358`
(`the_kind_selectors_two_directions_agree`), `:1242`
(`the_hand_built_rows_are_the_ones_the_table_leaves_out`);
`main.rs:5970` (`the_runner_row_is_hidden_exactly_when_it_cannot_be_disabled`)
asserts the *drawn* form, which is why this is (A) rather than a source reading.

### P-20 — Executable browse with `*.exe` filter; auto-fills name — **(C)**

**The picker messages are empty arms and nothing emits them.**

```rust
Message::PickExeFile { field: _field } => {}
Message::ExeFileChosen { field: _field, path: _path } => {}
```

(`main.rs:1443`, `:1446`.) There is no file dialog in the port. The second clause
— auto-filling the name from the basename — exists as form logic but is
unreachable, because the first clause can never fire.

### P-21 — Share URLs resolved on save *and* on browse — **(A), with a dead half**

The resolver is the best-tested thing in the tree on this subject:
`crates/core/src/netpaths.rs` carries
`only_a_remote_scheme_with_a_netloc_is_remote` (`:640`),
`a_file_url_is_unwrapped_and_percent_decoded` (`:885`),
`a_mounted_share_resolves_onto_its_gvfs_path` (`:898`),
`an_unmounted_share_comes_back_unchanged_so_the_error_can_name_it` (`:954`),
`an_unmatched_mount_name_is_found_by_scanning_the_gvfs_root` (`:975`), plus the
`unquote_matches_cpython` (`:576`) family. The save path reaches it
(`state.rs:476`, `main.rs:2603`). **The "on browse" clause is dead** — browse is
P-20, which is (C). Recorded here rather than folded in, because the resolver is
genuinely met and the failure is elsewhere.

### P-22 — Launch arguments, working directory, free-text fields — **(A)**

Evidence: `crates/core/src/runners/launch_opts.rs:969`
(`the_documented_examples_parse_as_python_does`), `:1009`
(`a_backslash_is_not_an_escape_character_here`), `:1082`
(`an_empty_text_is_empty_rather_than_a_single_blank_key`), `:1804`
(`malformed_arguments_are_an_error_rather_than_a_silent_drop`);
`crates/core/src/installers.rs:2490`
(`extra_arguments_are_shlex_split_not_whitespace_split`). The `shlex.split`
requirement in the Verify column is met assertion-for-assertion.

### P-23 — Editable category combobox (preset list + custom) — **(A)**

Evidence: `crates/app/src/view/form.rs:1311`
(`form_categories_are_the_built_ins_then_the_libraries_own`). The preset list is
`crates/core/src/covers.rs:43` (`DEFAULT_CATEGORIES: [&str; 13]`).

### P-24 — Cover preview + Find cover + custom image import — **(C)**

**All three paths are unimplemented or dead.**

- *Import*: `Message::PickCoverFile => {}` (`main.rs:1448`) and
  `Message::CoverFileChosen(_path) => {}` (`:1450`) — empty arms, no dialog, no
  producer.
- *Find cover*: `Message::FetchCoverForForm { .. } => {}` (`main.rs:1809`) and
  `Message::FormCoverFetchFinished { .. } => {}` (`:1818`), both empty, with a
  `TODO(T-11)` naming the reply-token check they also lack. `main.rs:5920`
  (`the_find_cover_button_is_drawn_iff_its_message_is_handled`) is the honest
  record of this: it asserts `button_on_screen == !COVER_FETCH_MISSING` **and**
  `handled == !COVER_FETCH_MISSING`, i.e. the button is on screen exactly while
  its message does nothing.
- The banner case ("Find cover on Half-Life 2") is moot: **there is no cover
  fetching in the port at all** — see P-60.

### P-25 — Steam genre auto-categorises game when Uncategorized — **(C)**

Unreachable for the same reason: it happens inside the cover lookup
(`bridge.py:553-554`), and `FetchCoverForForm`/`FormCoverFetchFinished` are
empty arms (`main.rs:1809`, `:1818`).

### P-26 — Runner picker + optional per-game prefix — **(A)**

Evidence: `crates/app/src/view/form.rs:1391`
(`the_runner_selection_carries_the_id_and_not_the_label`) — the id-not-label
half is the defect class this project has produced twice — and
`crates/core/src/runners/launch.rs:1231`
(`an_empty_prefix_path_falls_back_to_the_prefixes_directory_for_the_game_id`)
for the "empty = isolated `<prefixes>/<id>`" clause.

### P-27 — All 15 per-game toggles — **(A)**

Evidence, form side: `crates/app/src/view/form.rs:1162`
(`the_toggle_tables_are_the_reference_pages_switches`), `:1242`, `:2137`
(`a_switch_sends_the_name_of_its_own_row`). Environment side:
`crates/core/src/runners/launch_opts.rs:1367`
(`the_sdl_toggle_sets_the_hidapi_variables_only_for_proton`), `:1383`, `:1403`,
`:1413` (`esync_and_fsync_write_both_directions_including_the_negative_one`),
`:1436`, `:1465`, `:1475`, `:1489`, `:1521`, `:1534`, `:1577`, `:1604`, `:1620`,
`:1632`.
**Not covered:** "relaunch shows env effect" — the *mapping* is asserted; delivery
to a live process is P-42's residual.

### P-28 — Virtual-desktop size field, validated, default 1920x1080 — **(A)**

Evidence: `crates/app/src/view/form.rs:2171`
(`the_desktop_size_field_needs_both_gates`); the wrapper and the default in
`crates/core/src/runners/launch_opts.rs:1577`
(`the_virtual_desktop_wrapper_is_windows_only_and_nests_inside_the_wrappers`),
with the parsing tests at `:1219` (`a_valid_resolution_is_normalised_and_anything_else_falls_back`)
and `:1290` (`only_ascii_spaces_are_removed_not_every_kind_of_space`).

### P-29 — Additional app launched alongside in same prefix — **(B)**

The code exists — `crates/core/src/runners/launch.rs:364-375` builds the extra
argv, prepends the runner executable, and detaches it with the same environment
("Detached and unreaped, exactly as Python leaves it", `:370`) — and its doc at
`:310` says it starts *before* the main process. **No test mentions
`additional_app`**: the only two occurrences in `launch.rs` are that doc line and
that code line. So "both processes start" is asserted by nothing.

Procedure: create a game whose exe is a script that writes a file and sleeps;
set `Additional app` to a second script that does the same; press Play. Pass =
both files appear, the helper's process is not reaped by the app, and killing
the app leaves the helper. Fail = one file, or the helper dies with the title.

### P-30 — Custom `KEY=value` env block, quoting/`;`, overrides toggles — **(A)**

Evidence: `crates/core/src/runners/launch_opts.rs:1118`
(`the_environment_block_round_trips_through_a_realistic_block`), `:1145`
(`an_unquoted_semicolon_splits_the_line_into_two_assignments`), `:1109`
(`a_later_assignment_wins_over_an_earlier_one`), `:1168`
(`merging_an_override_never_loses_the_existing_one`), `:1679`
(`the_environment_block_is_applied_last_so_the_user_wins`), `:1762`
(`an_empty_environment_block_leaves_the_toggles_alone`). The
`FOO="a;b"` case named in the Verify column is the `:1145` pair.

### P-31 — Saving without cover triggers auto-fetch — **(C)**

**No producer.** The save path (`main.rs`, `Message::SaveGameForm`) stores the
game and toasts; nothing constructs `FetchCover`, and `Message::FetchCover`
(`main.rs:1805`) plus `Message::CoverFetchFinished` (`:1806`) are empty arms
with no constructor in any view. Doubly moot: no cover fetching exists (P-60).

---

## 2.3 Runners

### P-32 — Installed list: System Wine + downloaded builds with family label — **(A)**

Evidence: `crates/app/src/view/runners.rs:1164`
(`a_release_is_installed_exactly_when_its_own_directory_is_on_disk`) and `:1209`
(`a_directory_without_a_proton_entry_is_not_an_installed_build`) — the second is
the reference's own subtlety. `:1233`
(`a_release_detail_names_the_family_the_asset_and_the_whole_megabytes`) covers
the "GE-Proton9-5 · Proton-GE" shape. **Not covered:** the "Not installed"
variant of the System Wine row.

### P-33 — Download families: 8 + System Wine guide row — **(C)**

The *rows* are met and asserted — `crates/app/src/view/runners.rs:1345`
(`the_guide_has_one_row_per_family_plus_system_wine`, `rows.len() == 9`), `:1326`
(`a_guide_subtitle_is_the_kind_and_the_maintainer_or_just_the_kind`). **The
homepage buttons are dead.** `view/runners.rs:450` builds the guide card's
link as `button::link(family.homepage())` and `:581` builds "Visit project" the
same way — both **without `on_press`**; the file's own comment at `:578` records
that "a `Button` with no `on_press` renders **disabled**" (verified in the
vendored toolkit: `libcosmic/src/widget/button/link.rs` → `Button::new(label,
Hyperlink{..})` leaves `on_press: None`, and `button/widget.rs:148` renders that
disabled). So the button the item promises cannot be pressed.

**The test is the defect.** `every_guide_row_that_has_a_homepage_names_one_a_button_can_open`
(`view/runners.rs:1355`) is named "…a button **can open**" and asserts only
`row.homepage.starts_with("https://")`. A green run of that test is the exact
defect class this project keeps producing: the instrument (a string prefix) is
narrower than the claim in its own name. It must either assert `on_press`, or be
renamed.

### P-34 — Per-family release list (≤12), tag + asset + MB + Installed chip — **(A)**

Evidence: `crates/app/src/view/runners.rs:1515`
(`a_fetch_returns_at_most_pythons_twelve_releases`), `:1233`, `:1243`
(`a_half_megabyte_rounds_up_as_python_rounds_it`), `:1164`.
`:1614` (`a_reply_for_a_family_the_user_has_left_is_dropped`) and `:1655`
(`a_row_reply_for_a_superseded_request_is_dropped`) cover the stale-reply half.

### P-35 — Download with progress bar, busy-guard, completion toast — **(A)**

Evidence: `crates/app/src/view/runners.rs:272` (`pub fn progress_fraction`), with
`:1379` (`no_job_means_no_progress_bar`), `:1389` (`a_busy_job_with_no_fraction_yet_draws_no_bar`),
`:1400` (`a_busy_job_with_a_fraction_draws_the_bar_at_that_fraction`), `:1414`
(`the_easy_install_job_moves_this_pages_bar_as_well`), `:1752`
(`a_progress_tick_is_stored_and_the_bar_rule_decides`); the guard at `:1819`
(`a_click_while_an_install_is_running_is_dropped`) and `:1847`
(`installing_marks_the_page_busy_at_zero_progress_before_the_work_starts`); the
completion at `:1726` (`an_install_finishing_clears_the_guard_on_success_and_on_failure`).
Retracting my earlier note: the bar reads `progress_fraction`, not a raw field.

### P-36 — Release asset filtering (require/exclude/prefer tokens) — **(A)**

Evidence: `crates/core/src/runners/families.rs:296`
(`asset_matches_tokens`) with the negative cases at `:642-667`, whose comment
names `wow64`, `v3` and `znver4` as the tokens the filter exists to exclude —
exactly the Verify column's "CachyOS list has no `v3`/`znver4` assets";
`crates/core/src/runners/proton.rs:1624`
(`a_family_excludes_the_assets_its_token_list_rejects`).

### P-37 — Runner removal with confirm; games fall back to System Wine — **(C)**

Code at HEAD (`afec332`) implements the dialog: `ConfirmRemoveRunner`
carries id+name into `confirm_remove_runner` (`view/runners.rs:1049-1055`),
`remove_runner_dialog` titles `"Remove {name}?"` with the fixed subtitle
(`main.rs:1096-1111`), Remove sends `RemoveRunnerConfirmed` which clears the
pending state before removing (`:1061-1064`).

**Live-walk status 2026-09-12: NOT verified — the running build predates the
dialog.** Instrument evidence: rail clicks navigate (ndiff 82037–434892) but
16 content clicks over the delete-button area (x1100–1240 × y306–420,
park-then-shoot discipline) all return ndiff 0; dropdown/release/guide clicks
also ndiff 0. The deployed binary
(`.../stable/active/files/bin/gamehandler`, the only deployed generation,
running pid 1592061 since 05:02:54) contains the dialog subtitle string —
but `git log -S "deleted from disk"` shows the string predates `afec332`
(the commit only *moved* it into `remove_runner_subtitle()`), while the
button's message changed `UninstallRunner` → `ConfirmRemoveRunner` in that
same commit (`974d875:runners.rs:731` still sends `UninstallRunner`). A click
that lands the old message removes the runner directly with no dialog — and
`GE-Proton-testwalk` is still on disk, consistent with clicks landing
nowhere at all. Either way the live build cannot verify the dialog. **#89
(rebuild + reinstall from HEAD) is the gate; re-walk P-37 after it.**
Clause-1 oracle `shots/172-dialog.png` (md5 `b7ecc142`) kept for the re-walk.

The stale pre-dialog analysis below is superseded by the above; kept for
history.

Two clauses, both unmet. **No confirm dialog:** `grep -n "confirm\|Confirm"
crates/app/src/view/runners.rs` returns nothing. **The Installed list is a
pre-deletion snapshot:** the `UninstallRunner` arm (`view/runners.rs:997-1007`)
computes `let rows = refresh(state);` at `:1001` and *then* calls
`uninstall_line(runner_id, proton::uninstall(...))` at `:1002-1005` — and
`refresh` (`:367`) reads `state.runners.installed_protons()` synchronously at
`:370`, one statement before its own `Task::perform`. So the removal the user
just performed is not reflected in the list they are looking at. The reference's
"games fall back to System Wine" is a selection-time behaviour with no test
here.

### P-38 — Secure extraction: traversal, symlink confinement, caps, atomic rename, staging cleanup — **(A)**

Evidence: `crates/core/src/runners/archive.rs:938`
(`a_parent_traversal_member_is_refused`), `:953`
(`a_traversal_that_normalises_back_inside_is_allowed` — the false-positive
guard), `:966` (`a_symlink_escaping_the_destination_is_refused`), `:1339`
(`a_symlink_that_escapes_once_moved_is_refused`), `:1358`
(`an_absolute_symlink_in_a_staged_tree_is_refused`), `:1371`
(`a_relative_symlink_that_stays_inside_is_allowed`), `:1102`
(`the_member_limit_stops_streaming_inside_private_staging`);
`crates/core/src/runners/proton.rs:2532` (`a_top_level_symlink_refuses_the_archive`),
`:2336` (`a_broken_symlink_at_the_target_is_not_silently_replaced`), `:2764`
(`the_staging_directory_is_private_while_it_exists`). The reference's
`test_security.py:97-190,266-420` is superseded assertion-for-assertion.

### P-39 — Runner tags sanitised to collision-resistant install ids — **(A)**

Evidence: `crates/core/src/runners/archive.rs:1257`
(`release_tags_are_sanitised_before_becoming_install_paths`) asserting the
`release-v1~h` prefix and an **exact 12-hex-character** suffix length at `:1261`
— the collision-resistance clause, asserted rather than described; plus
`crates/core/src/runners/families.rs:951`
(`a_tag_that_needs_sanitising_still_produces_a_safe_directory`).

---

## 2.4 Launch semantics

### P-40 — Proton via `umu-run` with `PROTONPATH`/`GAMEID`/`STORE`; plain-wine fallback — **(A)**

Evidence: `crates/core/src/runners/mod.rs:1476` —
`assert_eq!(pure_posix_name(&command.argv[0]), "umu-run")` — and `:1516` —
`assert!(!command.env.contains_key("PROTONPATH"))` for the fallback. The
fallback's own tests are the `pure_posix_name` family at `:1638-1642`.
**Not covered:** `STORE` specifically is not named in an assertion I read; the
`PROTONPATH`/`GAMEID` pair is.

### P-41 — Proton-only gates: NVAPI, FSR, Wayland raise without umu+proton — **(A), by source**

The gates are at `crates/core/src/runners/launch.rs:339-345`:

```rust
if game.nvapi && !uses_proton { return Err(RunnerError::NvapiNeedsProton); }
if game.fsr && !uses_proton { return Err(RunnerError::FsrNeedsProton); }
if game.wayland && !uses_proton { return Err(RunnerError::WaylandNeedsProton); }
```

The routing *into* `uses_proton` is tested —
`crates/core/src/runners/launch.rs:1398`
(`nvapi_follows_the_proton_runtime_rather_than_the_family_name`) — and the
command builder's twin gates are tested at
`crates/core/src/runners/launch_opts.rs:1489`
(`nvapi_and_fsr_require_proton_features`). **What no test observes is a launch
that returns one of these three errors**, i.e. the Error → toast path named in
the Verify column ("error toast, no launch"). Bucketing this as (A) is a
judgement and it is the weakest (A) in this list: the gate is real and readable,
its observable consequence is unasserted.

### P-42 — Esync/Fsync/DXVK-off/VKD3D-off env mapping incl. `PROTON_NO_*` and dll overrides — **(A)**

Evidence: `crates/core/src/runners/launch_opts.rs:1413`
(`esync_and_fsync_write_both_directions_including_the_negative_one`), `:1436`
(`the_dxvk_toggle_off_adds_wined3d_and_keeps_the_inherited_overrides`), `:1465`
(`the_vkd3d_toggle_only_appends_an_override`), `:1475`
(`both_dll_toggles_off_append_in_python_s_order`), `:1741`
(`the_caller_s_environment_is_never_mutated`). The Verify column's "inspect env
of launched process" is met at the mapping boundary; that the map reaches the
child is `Process::envs` and is not separately asserted.

### P-43 — MangoHud / GameMode / Gamescope incl. `--hdr-enabled` and the missing-binary error — **(A)**

Evidence: `crates/core/src/runners/launch_opts.rs:1604`
(`mangohud_prefers_the_binary_and_falls_back_to_the_variable`), `:1620`
(`gamemode_wraps_only_when_gamemoderun_is_installed`), `:1632`
(`gamescope_wraps_outermost_and_only_carries_hdr_when_it_is_on`), `:1665`
(`a_missing_gamescope_is_an_error_naming_the_flathub_extension`) — and the last
asserts the Flathub extension id verbatim:
`assert!(message.contains("org.freedesktop.Platform.VulkanLayer.gamescope//25.08"))`
(`:1674`).

### P-44 — Anti-cheat runtimes auto-located, empty-string disable — **(A)**

Evidence: `crates/core/src/runners/launch_opts.rs:1521`
(`a_disabled_anticheat_runtime_is_an_empty_value_not_a_missing_key` — the
empty-string clause is the assertion, and it is the one the reference cares
about), `:1534` (`an_enabled_anticheat_runtime_uses_what_the_host_found`),
`:1560` (`the_anticheat_lookup_is_skipped_entirely_without_proton_features`);
the Flatpak Steam root at `crates/core/src/runners/env.rs:221` is covered by
`env.rs:501` (`anticheat_search_prefers_an_extra_root_over_the_built_in_list`).

### P-45 — Bundled DXVK installed into raw-Wine prefixes once per version — **(A)**

Evidence, all in `crates/core/src/runners/launch_opts.rs`: `:1818`
(`dxvk_fixture`), `:1846` (`a_prefix_without_wineprefix_is_refused_before_anything_is_read`),
`:1866` (`an_incomplete_bundle_is_refused_rather_than_half_installed`), `:1880`
(`a_64_bit_prefix_gets_both_architectures_in_both_directories`), `:1913`
(`a_win32_environment_installs_one_architecture_into_system32`), `:1939`
(`an_old_prefix_records_its_architecture_in_the_registry`), `:1998`
(`the_version_marker_makes_a_second_install_a_no_op` — the "once per version"
clause), `:2021`, `:2040`, `:2058`, `:2077`, `:2094`, `:2111`. Also
`launch.rs:1455` (`the_game_environment_block_reaches_the_dxvk_installer_and_not_only_the_child`).

### P-46 — Immediate-failure detection: 6 s grace, stderr tail, toast + window restore — **(B)**

The detection half is well tested in core: `crates/core/src/runners/launch.rs:912`
(`a_silent_non_zero_exit_falls_back_to_the_status_line`), `:931`
(`a_title_still_running_at_the_deadline_is_not_a_failure` — the grace window's
positive control), `:946` (`wine_noise_is_filtered_out_of_the_reported_error`),
`:999` (`the_tail_buffer_trims_from_the_front_to_its_limit`), `:870`
(`a_descendant_holding_stderr_neither_deadlocks_the_drain_nor_loses_the_text`);
and in the shell, `main.rs:4736`
(`the_grace_watch_reports_only_when_the_title_died`) and `main.rs:3432`
(`a_launch_that_stops_right_away_exits_non_zero_and_still_records_it`).

**Two defects, both found by looking for the instrument.**
1. `LAUNCH_GRACE_SECONDS` (`crates/core/src/runners/mod.rs:91`, `= 6.0`) is
   asserted by **nothing** — a grep for the constant returns its definition and
   no test. The Verify column's "6 s grace" is a number no instrument pins.
2. `main.rs:3428` claims the constant is "pinned by core's own tests". That
   sentence is false, and a doc comment asserting coverage that does not exist
   is worse than silence because the next reader stops looking.

Procedure (the item's own Verify): install a game whose exe is `exit 1`; enable
*Close on launch*; press Play from the library. Pass = the window closes, then
returns, with a toast naming the failure and carrying the child's stderr tail.
Fail = the window stays hidden, or the toast says nothing about why. Also
observe the *timing*: the toast must not appear before ~6 s for a title that
exits immediately, which is the only way to see the grace value from outside.

### P-47 — `mark_played` timestamp + "Launching …" toast — **(A)**

Evidence: `crates/app/src/main.rs:3384`
(`a_real_launch_records_the_game_as_played`), which drives the real path, and
`main.rs:3432` (the same recording on a launch that dies immediately). The
"Launching …" toast is a `toast_task` at the same site.

### P-48 — Winecfg/Winetricks run with the game's own WINE+WINESERVER — **(A), unreachable**

The core claim is met and asserted: `crates/core/src/runners/launch.rs:1105`
(`winecfg_uses_the_games_own_wine_and_prefix`) and `:1156`
(`wineserver_is_set_only_when_it_sits_beside_wine_and_is_executable`) — the
second clause is the reference's own condition. **Reachability is (C)**, exactly
as P-13: no view constructs `Message::RunPrefixTool`.

### P-49 — Open prefix folder (Proton `pfx` layout aware) — **(A), unreachable**

Evidence: `crates/core/src/runners/launch.rs:1136`
(`the_prefix_is_the_proton_subdirectory_when_one_exists`) — the `pfx` awareness
the Verify column names. Reachability is (C), per P-13.

### P-50 — Linux-native launch (no Wine env at all) — **(A)**

Evidence: `crates/core/src/runners/launch.rs:1263`
(`a_linux_title_is_started_with_its_own_environment_and_no_runner`) and
`crates/core/src/runners/launch_opts.rs:1771`
(`a_native_command_carries_the_inherited_environment_untouched`), plus `:1794`
(`a_native_command_with_no_executable_is_an_error`) and `:1698`
(`a_windows_toggle_is_the_only_thing_gated_on_the_game_s_kind`).

---

## 2.5 Easy installers

### P-51 — Catalog of 9: 8 launchers + Discord under Apps — **(A)**

Evidence: `crates/core/src/installers.rs:2201`
(`the_catalog_is_eight_launchers_and_one_app`), `:2224`
(`every_recipe_is_complete_enough_to_be_used`), `:2260`
(`epic_is_the_one_msi`); page side `crates/app/src/view/installers.rs:1270`
(`the_catalog_is_the_core_catalogs_recipes_in_its_order`) and `:1211`
(`every_category_the_catalog_can_produce_is_one_the_filter_offers`).

### P-52 — Search + Launchers/Apps filter — **(A)**

Evidence: core `crates/core/src/installers.rs:2283`
(`search_matches_by_id_and_the_category_filter_narrows`), `:2300`
(`search_reaches_the_name_the_description_and_the_id`), `:2336`
(`the_category_filter_is_exact_and_never_folded`), `:2347`
(`filtering_preserves_the_catalog_order`); page side
`crates/app/src/view/installers.rs:1353`
(`the_search_box_and_the_category_filter_reach_the_catalog`), `:1189`, `:1245`.

### P-53 — Per-install runner choice, isolated prefix per install — **(A) — MET, and it was not**

The first clause was finding #97 and **was fixed at `f0231c6`** during this
pass. I re-measured it in the sandbox at `9bcacf9` with a probe on the drawn
dropdown: row 1 emits `SetInstallRunner("GE-Proton9-5")` — the *id*, where the
old form emitted the *index* — and `settings.default_runner` is left unchanged
at `"wine-system"`. The guards are now assertion-level:
`crates/app/src/view/installers.rs:883`
(`the_runner_selection_carries_the_id_and_not_the_position`), `:1049`
(`choosing_a_runner_for_the_next_install_leaves_the_global_default_alone` — the
wrong-destination half), `:1084`
(`the_chosen_runner_is_the_one_the_install_press_names`), and
`crates/app/src/view/form.rs:1954`
(`no_dropdown_callback_turns_its_index_into_the_payload`), which is a
source-scanning guard with its own positive control at `:1914`
(`the_dropdown_guard_reports_the_defect_it_was_written_for`).
The prefix clause: `crates/core/src/installers.rs:2949`
(`prepare_prefix_creates_and_returns_the_games_directory`).

**Consequence for the record:** §2's Verify column for P-53 cites
`bridge.py:840-846,649-653` — Python. The item is met, but not by the evidence
the standard names.

### P-54 — Download (≤1 GiB) with origin-allowlist + magic-byte + Authenticode — **(A) for the checks; the app-wiring half is (C)**

This is the item I was asked to confirm as (C) and it needs splitting, because
the honest answer is that the **core is exceptionally well tested and the
wiring is not tested at all**.

Core, all in `crates/core/src/installers.rs`:
`:3299` (`an_authenticated_download_is_installed_atomically`), `:3350`
(`an_untrusted_redirect_is_rejected_before_a_single_byte`), `:3400`
(`a_payload_that_is_not_an_exe_is_refused_before_the_signature`), `:3434`
(`a_payload_that_is_not_an_ole_file_is_refused_for_an_msi`), `:3490`
(`the_signature_must_name_a_publisher_the_recipe_approves`), `:3528`
(`a_verifier_that_did_not_say_ok_is_a_failure_with_its_own_output`), `:3557`
(`a_missing_verifier_is_an_error_that_names_the_dependency`), `:3577`
(`a_recipe_that_pins_the_microsoft_root_passes_it_to_the_verifier`), `:3640`
(`a_declared_length_over_the_cap_is_refused_before_the_body`), `:3690`
(`a_body_that_crosses_the_cap_is_refused_while_it_streams`), `:3755`
(`progress_is_reported_as_a_fraction_and_ends_at_one`), `:3811`
(`a_refused_download_leaves_an_existing_installer_alone`), plus the URL-parser
family at `:3006` (`the_url_parser_matches_cpython`) and `:3149`
(`the_origin_check_accepts_the_recipes_host_and_refuses_another`). The
`osslsigncode` tool is in the manifest (`build-aux/flatpak/com.goshapps.GameHandler.json:38-54`),
so the sandbox has it.

**(C) residual — the wiring.** `easy_install_worker` and its spawn are asserted
by nothing. In the previous session I mutated that worker to delete the
download-failure report and the full workspace suite stayed green, with
`Compiling` observed so the binary was fresh (finding-class #50). So "Tampered
binary → nothing executed" is true of the *downloader* and unproven of the
*app*.

### P-55 — msi via `msiexec /i`, exe directly; no Proton-only vars under raw Wine — **(A) for the mapping; (C) for the wiring**

Evidence: `crates/core/src/installers.rs:2457` (`an_msi_is_run_through_msiexec`),
`:2465` (`an_exe_is_handed_straight_to_wine`), `:2553`
(`a_raw_wine_install_run_gets_no_proton_only_variables` — the clause the Verify
column names), `:2581` (`a_proton_install_run_keeps_proton_features`), `:2525`
(`the_installer_command_carries_the_prefix_and_the_sync_knobs`). Wiring: same
(C) as P-54.

### P-56 — Wizard wait: poll, wineserver-idle slicing, 6 h ceiling, profile search — **(A) for the core; (C) for the wiring**

Evidence, all in `crates/core/src/installers.rs`: `:3921`
(`the_poll_waits_until_the_wizard_writes_the_executable`), `:3932`
(`the_poll_gives_up_at_the_deadline`), `:3956`
(`a_wineserver_wait_that_proves_something_shortens_the_window`), `:3978`
(`an_already_installed_executable_is_returned_after_the_handoff_only` — the
"after the handoff only" clause), `:3998`
(`the_launcher_is_found_while_wineserver_is_still_busy`), `:4041`
(`the_wineserver_beside_the_runners_wine_wins_over_the_one_on_path`); the
search: `:2662` (`a_path_resolves_case_insensitively`), `:2683`
(`a_users_path_is_retried_under_every_profile`), `:2701`
(`a_relocated_executable_is_found_by_its_basename`), `:2730`
(`the_fallback_finds_a_relocated_executable`), `:2744`
(`the_fallback_scan_stops_at_the_depth_limit`), `:2762`
(`the_fallback_scan_does_not_enter_the_windows_directories`); the `pfx`
layout `:2844`/`:2868`. Wiring: the same untested spawn as P-54.

### P-57 — Fallback "Locate …" dialog; cancel keeps prefix — **(C)**

**Confirmed, and worse than a missing dialog.** The not-found branch
(`main.rs:2543-2565`) toasts "Could not find the {} executable in the
prefix{suffix}. Pick it yourself if the install finished.", sets
`state.running_install = None`, and inserts into `state.easy_pending`. But:

- the arm's own doc at `main.rs:2543` records that it **leaves `easy_busy`
  true**;
- `easy_busy` gates the whole page (`main.rs:2374`: `if state.easy_busy { return
  toast_task("Another install is already running") }`);
- **nothing produces `Message::CompleteEasyInstall` or
  `Message::CancelEasyInstall`** (no constructor in any view), and
- **`easy_pending` is never read** after being written.

So a user whose wizard closed without installing is left with a page that
refuses every subsequent install, no locate dialog, and no cancel. The state
machine's three transitions exist and are tested
(`main.rs:4993` `a_wizard_that_found_no_executable_stores_the_install_and_stays_busy`,
`:5057` `a_cancel_clears_the_guards_and_keeps_the_prefix`, `:5187`
`a_located_executable_finishes_the_install_and_an_empty_path_cancels_it`) —
which is exactly why the dispatch-coverage guards cannot see it: the arms are
handled and correct. Only the *producers* are missing.

### P-58 — Finished install becomes library entry with exe-icon cover and Play toast — **(C), one clause met**

Met: `main.rs:5137` (`a_wizard_that_found_the_executable_makes_a_library_entry`)
and `main.rs:5252` (`the_installed_toast_offers_play_and_moves_to_the_library`).
Unmet: **the cover is not the vendor icon** — there is no exe-icon extraction
anywhere in the port (see P-61), so a finished install gets whatever the
generic cover path produces. `The_installed_toast`'s "Play" clause is also
weaker than its name: it asserts the action's *description* inside the toast's
`Debug` output (which I verified is real —
`Action<Message>`'s manual `Debug` prints `self.description`), and nothing
drives the closure, so "offers Play" is proven and "pressing it navigates" is
not.

### P-59 — Concurrent-install guard — **(A)**

Evidence: `crates/app/src/main.rs:4912`
(`an_install_in_flight_refuses_a_second_one`) and the guard itself at `:2374`
(`"Another install is already running"`); `state.rs:972`
(`self.runner_busy || self.easy_busy`) covers the cross-path case,
`:1012`/`state.rs:1091` the idle representation.

---

## 2.6 Covers

**This whole group is the most serious finding of the pass.** `crates/core/src/covers.rs`
contains, in full: `COVER_ACCENTS: usize = 8` (`:27`), `DEFAULT_CATEGORIES`
(`:43`), `accent_index` (`:77`) and `accent_index_in` (`:94`) — the gradient
bucket. Its tests are the eight shade tests at `:111-194`. **There is no Steam
search, no match scoring, no CDN fallback list, no download, no size cap, no
empty-download rejection, and no PE/icon parser anywhere in `crates/`**: greps
for `steampowered`, `searchfilter`, `library_600x900`, `header.jpg`, `RT_GROUP_ICON`,
`extract_icon`, `MAX_DOWNLOAD` and `12 * 1024` over the whole workspace return
nothing outside `installers.rs`'s unrelated downloader and a credits URL.

### P-60 — Steam search → scored match → portrait cover with CDN fallbacks — **(C)**

Not implemented. `covers.py:183-210,333-356` has no counterpart.

### P-61 — Offline exe-icon fallback; store launchers get vendor icon — **(C)**

Not implemented — no PE parser exists. Consequence for P-58.

### P-62 — Generated initials plate, 8 stable gradients, letterboxed icons — **(A)**

Evidence, all in `crates/app/src/view/cover.rs`: `:389`
(`initials_match_the_reference`), `:401` (`initials_of_nothing_are_a_question_mark`),
`:416` (`initials_split_on_ascii_only_like_the_reference`), `:424`
(`a_one_letter_first_word_is_not_doubled`), `:348`
(`icons_letterbox_and_photographs_fill` — the letterbox clause), `:358`
(`only_the_plate_lacks_artwork`), `:367` (`the_shade_wraps_and_is_total`),
`:377` (`every_gradient_has_two_distinct_non_black_stops`); stability
`:444` (`the_plate_shade_is_pythons_hash_bucket`), `:465`,
`:476`; and the magic-byte sniffing that decides icon-vs-photo at `:272-338`
(including `:281` `webp_bytes_in_a_jpg_named_file_are_a_photo`). **Not covered:**
"stable across restarts" is asserted as a pure function of the seed, which is
the right way to assert it.

### P-63 — Download size caps (12 MiB), empty-download rejection, atomic writes — **(C)**

Not implemented — there is no cover download to cap. (The *installer*
downloader's caps are P-54's and are met.) Note the reference's
`test_covers.py:65-83` cited in §2 tests the Python app.

---

## 2.7 Plugins / Credits / Settings / Shell / CLI

### P-64 — Plugins page: 5 helpers, 3 states, Flatpak never offers host install — **(A)**

Evidence, all in `crates/core/src/plugins.rs`: five `Plugin(` entries at
`:92,106,119,132,144`; `the_catalogue_is_the_reference_catalogues_data` (`:644`)
and `the_catalogue_has_the_reference_counts_and_order` (`:677`) both read the
*count from the reference source* rather than restating a literal;
`the_three_states_are_reachable_and_distinct` (`:995` — each state from a
different fake host, so a collapsed branch fails);
`flatpak_never_offers_host_package_commands` (`:960`) asserts
`install_command(.., Some("apt"), ..) == Err(InstallError::Flatpak)` **and**
`error.to_string().contains("unavailable inside Flatpak")` — so the refusal is
a policy, not a detection fallback, which is the clause that matters;
`the_flatpak_marker_is_either_the_variable_or_the_file` (`:978`) covers both
signals; `a_helper_with_no_package_mapping_is_unavailable_rather_than_missing`
(`:1050`). Page side: `crates/app/src/view/plugins.rs:344`
(`the_button_is_the_qmls_three_labels_and_only_missing_acts`), `:416`, `:380`.
**Not covered:** that the *install* runs. `the_runner_kills_a_child_that_outlasts_its_budget`
(`:510`) and `a_command_that_cannot_be_spawned_is_reported_and_not_panicked`
(`:538`) cover the runner, not a real `pkexec`.
Note `crates/app/src/view/plugins.rs`'s own TODO acknowledges the
`button::link`-disabled pattern (`:578`) — the *same* defect as P-33's homepage
buttons.

### P-65 — About & Credits: 5 sections, 25 entries, licenses, Visit buttons — **(C) at HEAD; (A) once the in-flight change lands**

**The catalogue is met:** `crates/core/src/credits.rs:580`
(`the_table_has_the_reference_counts`) derives the expected counts by counting
`CreditSection(` and `Credit(` **in `gamehandler/credits.py`**, so "5 sections /
25 entries" is the reference's number and not the test author's; `:615`
(`every_credit_carries_a_role_and_a_link`), `:632`, `:703`
(`a_licence_is_shown_only_where_the_reference_records_one`), `:527`
(`the_generated_readme_contains_this_modules_markdown_byte_for_byte`).

**The Visit buttons were dead at HEAD** (`ae90dab`): `Message::OpenUrl` does not
exist (`git show HEAD:crates/app/src/main.rs | grep -c OpenUrl` → 0), the page
recorded it honestly as `pub const LINKS_OPEN: bool = false` in
`crates/app/src/view/credits.rs`, and `the_links_are_still_unwired` (`:665`)
asserted `!links_open` **and** `!main_rs.contains("OpenUrl")` — a
self-invalidating test that fails the moment the fix lands, which is the right
shape for a record of an admitted gap. I verified the mechanism in the vendored
toolkit rather than from the comment: `libcosmic/src/widget/button/link.rs`
leaves `on_press: None`, and `button/widget.rs:148` renders that disabled.

**In the working tree at `ae90dab`+dirty:** `view/credits.rs:271` and `:337` now
carry `.on_press(Message::OpenUrl(..))`, `main.rs` has 6 `OpenUrl`
occurrences, and new tests at `credits.rs:676`
(`every_visit_button_opens_its_own_credits_url`) and `:720`
(`the_footer_button_opens_the_repository_url`) drive a real press through
`Widget::update` and match the published message. When that commits, P-65's
button clause becomes (A). What it will still not cover: the browser actually
opening.

**A defect in that in-flight change — measured, not predicted.**
`crates/app/src/view/form.rs:48` still doc-links
`` [`crate::view::credits::LINKS_OPEN`] ``, and the constant is gone from the
working tree (`grep -c "pub const LINKS_OPEN" crates/app/src/view/credits.rs`
→ 0; it is 8 occurrences at HEAD, const included). `scripts/verify.sh:562` runs
`stage_doc` under `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links"`, so I ran
that stage's command directly against the working tree:

```
$ RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc -p gamehandler --no-deps
error: unresolved link to `crate::view::credits::LINKS_OPEN`
  --> crates/app/src/view/form.rs:48:37
   |
48 | //! comments — the same device as [`crate::view::credits::LINKS_OPEN`] one module
   |                                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ no item named `LINKS_OPEN` in module `credits`
   = note: requested on the command line with `-D rustdoc::broken-intra-doc-links`
error: could not document `gamehandler`
EXIT=101
```

**The credits change as it stands in the working tree breaks `verify.sh`'s
doc stage.** The fix is to drop or rewrite the `form.rs:48` reference in the
same change; nothing else about the credits work is at fault.

### P-66 — Settings: schemes, grid/list, default runner, 13 default toggles, close-on-launch; persisted — **(C)**

**Confirmed unmet, and the strongest single finding of this pass.** The read
half is met: `crates/app/src/view/settings.rs:68` —
`pub const DEFAULT_TOGGLES: [(&str, &str, &str); 13]` — with
`the_toggles_are_the_reference_pages_toggles_in_order` (`:602`, which extracts
the keys from `SettingsPage.qml`), `every_toggle_in_the_table_reads_and_writes`
(`:535`), `a_repeated_write_changes_nothing_and_an_unknown_key_is_refused`
(`:573`), `every_selector_offers_only_values_the_model_accepts` (`:1178`), and
`crates/core/src/settings.rs:49`/`:52` for the two enums. The thirteen (not
fifteen) is correct and the source explains why `bridge.py:74-78`'s `wayland`
and `hdr` are dropped.

**Unmet: the words "restart; persisted in `settings.json`".** `Settings::save`
(`crates/core/src/settings.rs:178`) and `save_to` (`:183`) have no call site in
`crates/app` — see P-03 for the greps. Every mutation arm writes memory only:
`SetColorScheme` (`main.rs:1527`), `SetSortMode` (`:1542`), `SetDefaultRunner`
(`:1555`), `SetCloseOnLaunch` (`:1561`), `SetDefaultToggle` (`:1573`). The
comment at `main.rs:1560` even says "a no-op rather than a redundant save",
implying a save exists elsewhere; it does not. So a user who sets dark, or
MangoHud-by-default, or a default runner, loses all of it on quit.

Procedure to see it: Settings → turn on *Enable MangoHud by default* → quit →
relaunch → Settings. Pass = still on. Today it reads off.

### P-67 — Breeze-flavoured palettes; system scheme restores platform palette; Breeze icon fallback — **(C)**

**Two of the three clauses are not ported, and the source says so.** The mapping
is real and tested — `crates/app/src/theme.rs:114` (`theme_for(scheme, system)`
with `_ => system`), applied at `main.rs:2993` (init) and `main.rs:1527` (the
`SetColorScheme` arm) via `cosmic::command::set_theme`; `theme.rs:205`
(`light_and_dark_are_the_toolkits_own_and_not_the_system_theme`) and `:230`
(`system_defers_to_the_theme_it_is_handed_and_does_not_fall_back_to_dark`) use a
sentinel theme (`Theme::light_hc()`) so a `Theme::dark()` fallback fails the
assertion — that is a sound instrument.

What is unmet: the Breeze 13-colour `QPalette` (`theme.py:20-30`) and the
"system restores the platform palette" behaviour (`theme.py:78-88`) are
**deliberately not ported** — COSMIC's own `Theme::dark`/`light` stand in — and
`theme.rs`'s own note records that `system_preference()` has early returns to
`Theme::dark()` when the COSMIC config is unreadable (finding #58). So on a
non-COSMIC desktop, "system" **is** dark, which is exactly the failure the item
exists to prevent ("Force light on GNOME; no invisible text"). The file's own
honesty list at `theme.rs:142-147` also records mutations **F, G, H that
survive**: the arm storing without returning a task, init hardcoding `"dark"`,
and init returning no task at all — because a `Task` has no accessor. Those are
real surviving mutations and they mean the *plumbing* is unpinned even where the
mapping is pinned.

### P-68 — Shortcuts Ctrl+N / Ctrl+F / Ctrl+, / Ctrl+Q, listed in Settings — **(B)**

The mapping is a pure function and is tested —
`crates/app/src/shortcuts.rs:181`
(`each_own_accelerator_produces_the_reference_s_own_message`), `:213`
(`a_bare_key_activates_nothing`), `:230` (`an_extra_modifier_stops_the_match`),
`:249` (`logo_alone_is_not_control`), `:276`
(`the_keys_this_module_does_not_own_do_nothing`), `:297`
(`only_a_key_press_activates`), `:319` (`a_repeated_press_still_activates`) —
and the subscription is wired at `main.rs:2939`
(`cosmic::iced::keyboard::listen().filter_map(shortcuts::shortcut_for)`).
Ctrl+F is libcosmic's own `keyboard_nav` action reaching `App::on_search`
(`main.rs:3049`).

But the page's claim is asserted by a **hand-written constant comparison**:
`crates/app/src/view/settings.rs:679`
(`the_page_does_not_claim_a_shortcut_the_shell_does_not_implement`) asserts
`handles_keys == !IMPLEMENTED_SHORTCUTS.is_empty()` against
`IMPLEMENTED_SHORTCUTS: [&str; 4]` (`:156`) and `UNWIRED_SHORTCUTS: [&str; 0]`
(`:168`). That test cannot see whether `shortcut_for` returns `None` for a key
it advertises. So the claim "all four work" is not (A).

Procedure: focus the window on the Library page and press each of Ctrl+N, Ctrl+F,
Ctrl+, and Ctrl+Q; then repeat with the search field focused (the keys must pass
through, so typing `n` still types — that is the reference's
`Qt.ApplicationShortcut` behaviour and not a hole). **A documented divergence to
expect:** `Ctrl+Shift+F` also reaches `on_search`, because libcosmic's match
tests Control and does not reject Shift, where Qt's `Shortcut` would not match.
That is a fail against the reference, recorded in `shortcuts.rs`'s doc and not
printed on the page.

### P-69 — Toasts for every mutation; Play action; hide/restore on launch — **(C)**

**Met in machinery, unmet in coverage count.** `toast_task`
(`main.rs:2056`) is called from **25** sites in `main.rs` (plus 4 direct
`toasts.push(Toast::new(..))`, one of them `view/runners.rs:1035`), against the
reference's **39** `notify.emit` sites in `bridge.py`. So the word "every" in
the item is not met: 14 mutation sites that notify in Python are silent here.

The Play-action clause is proven only as a *label*: `main.rs:5252`
(`the_installed_toast_offers_play_and_moves_to_the_library`) reads the action's
description out of the toast's `Debug` string (valid — I checked the vendored
`Action<Message>` `Debug` impl — but the closure is never invoked).

Procedure: complete an easy install; press the toast's "Play"; pass = the app
navigates to the Library and the toast dismisses. Additionally, walk Python's
39 `notify.emit` sites and check each corresponding mutation for a toast; the 14
without one are the (C) list.

### P-70 — `--list`, `--launch ID`, `--version` work headless — **(A)**

**This is the one item whose evidence is exactly what the Verify column asks
for.** `scripts/verify.sh:733` (`stage_cli`) runs `cli-version`,
`cli-list-empty`, `cli-list-non-empty`, `cli-launch-unknown` and
`cli-launch-known` as real headless invocations of the built binary, dispatched
before anything GUI-shaped (`main.rs:98`, `:104-105`; DECISIONS D-12). Unit
coverage behind it: `main.rs:3075` (`list_lines_are_the_ids_and_names_in_name_order`),
`:3094`, `:3109`, `:3129`, `:3141`
(`launching_an_unknown_id_is_refused_with_pythons_message`), `:3156`, `:3191`,
`:3223` (`the_three_cli_sentences_are_pythons`). The reference's
`main.py:28-57,118-131` is superseded. **Not covered:** that `--launch <id>`
launches the *right* game — no test asserts the id-to-game binding at this
level.

### P-71 — Desktop shortcuts `gamehandler --launch <id>` — **(C)**

The writer is real and the escaping clauses are explicit —
`crates/core/src/runners/desktop.rs:110` (`escape_desktop_value`), `:126`
(`desktop_exec`), `:192` (`create_desktop_shortcut`), `:252` (mode
`| 0o111`), and the round trip `main.rs:3273`
(`a_shortcut_parses_back_into_the_launch_verb_and_the_games_id`), `:3312`
(`two_games_with_one_name_get_two_shortcuts`). **Unreachable, per P-14:** no
view constructs `Message::CreateDesktopShortcut`. This is #89 exactly —
"met-by-code and unreachable-by-user" — and it is the item's own Verify
("Shortcut file valid single-group entry, executable bit") that is unmet, since
no shortcut file can be produced by the app.

### P-72 — Network-share games via GVFS FUSE; unmounted share → instructional error — **(A)**

Evidence: `crates/core/src/netpaths.rs:143` (`unreachable_share_message`) with
`:1063` (`the_unreachable_message_names_the_host`) and `:1094`
(`the_unreachable_message_does_not_trim_its_input`); the resolver family at
`:640-1045`; the wiring `crates/core/src/runners/launch_opts.rs:843-849`, where
an unreachable share becomes the error's `message`, and
`crates/core/src/runners/launch.rs:1345`
(`an_unreachable_share_is_refused_before_the_process_starts`) — the "not
silence" clause, asserted at the point that matters. **Not covered:** a real
GVFS mount; a unit test cannot mount one. The message asserts; the FUSE
behaviour does not.

### P-73 — `games.json`/`settings.json` corruption tolerance + atomic saves — **(A) for tolerance; (C) for the settings write**

Tolerance, both files: `crates/core/src/settings.rs:367`
(`load_returns_defaults_for_every_broken_file`) asserts defaults across five
cases including `"{not json"` and `"[1,2,3]"`; `:395`
(`invalid_utf8_falls_back_to_the_defaults_instead_of_raising` — the reference
*raises* here, so this is a deliberate improvement, documented in the module);
`:457` (`a_leading_bom_is_stripped_rather_than_discarding_the_file`).
Atomicity: `crates/core/src/json.rs:354-366` (temp file beside the target, then
`rename`), asserted at `:827`
(`write_python_file_creates_parents_and_leaves_no_temporary`) and `:843`
(`assert_eq!(files, ["settings.json"], "the .tmp file must be renamed away")`).
`Library::save` is wired, so `games.json` really is written.
**The `settings.json` write is dead code** — P-66. So this row is (A) on
tolerance and (C) on "atomic saves of settings.json" in the running app.

### P-74 — Timestamp normalization (bool/string/negative/NaN/Inf) — **(A)**

Evidence: `crates/core/src/models.rs:873` (`int_timestamps_become_floats`),
`:887` (`invalid_timestamps_regenerate_added_and_zero_last_played`), `:910`
(`a_boolean_is_never_a_timestamp` — Python's `bool` is an `int`, so this is a
real trap and it is asserted), `:921` (`a_null_timestamp_is_treated_as_invalid`),
`:933` (`a_missing_timestamp_keeps_the_dataclass_default`), `:1011`; the
sanitiser at `crates/core/src/json.rs:213` with `:568`
(`sanitize_maps_non_finite_words_to_null`) and `:576`. The `"added": "soon"`
case in the Verify column is the `:887` family.

### P-75 — Unknown JSON keys ignored (forward compat) — **(A)**

Evidence: `crates/core/src/models.rs:860` (`unknown_keys_are_dropped`) and
`crates/core/src/settings.rs:312` (the same, for settings).

### P-76 — Categories list with Uncategorized last — **(A)**

Evidence: `crates/core/src/models.rs:1162`
(`categories_sort_uncategorized_last_then_alphabetically`), against the
implementation at `:698`. **Not covered:** the sort is `to_lowercase()` +
`dedup()`, not a collator, so a locale-dependent order is not guaranteed — and a
pair of categories differing only in case sorts adjacent and is collapsed by
`dedup()`. Neither is likely and neither is asserted.

### P-77 — Duplicate game ids: last write wins — **(A)**

Evidence: `crates/core/src/models.rs:1063`
(`duplicate_ids_keep_the_last_value_in_the_first_position`) — the second clause
("in the first position") is the part a naive dict port gets wrong and it is
asserted.

### P-78 — KDE Flatpak specifics: Gamescope extension PATH + VulkanLayer note; Steam anti-cheat roots; `~/.var/…Steam…:ro` — **(B)**

The manifest entries exist and are text-guarded:
`build-aux/flatpak/com.goshapps.GameHandler.json:23`
(`--filesystem=~/.var/app/com.valvesoftware.Steam/data/Steam:ro`) and `:24`
(`--env=PATH=/usr/lib/extensions/vulkan/gamescope/bin:...`), asserted by
`tests/test_packaging.py::test_flatpak_has_reviewed_launcher_permissions_and_multilib`
— but that test **parses the manifest JSON**, so it can see the flag and not the
granted permission. The core side: `crates/core/src/runners/env.rs:221` (the
Flatpak Steam anti-cheat root), `crates/core/src/runners/mod.rs:237` (the
VulkanLayer message, asserted at `launch_opts.rs:1674` naming
`org.freedesktop.Platform.VulkanLayer.gamescope//25.08`).

Procedure: on a KDE host, `flatpak install` the built bundle, then
`flatpak info --show-permissions com.goshapps.GameHandler` and confirm
`filesystem=~/.var/app/com.valvesoftware.Steam/data/Steam:ro` is granted;
`flatpak run com.goshapps.GameHandler` and confirm the window opens under a
KDE-looking theme; with Steam installed as a Flatpak, enable an anti-cheat
runtime and confirm the Flatpak Steam `steamapps/common` root is searched;
enable Gamescope with the extension absent and confirm the toast names
`org.freedesktop.Platform.VulkanLayer.gamescope//25.08` from Flathub.

---

## 3. The (C) list, with its causes

| ID | Defect | Cause |
|----|--------|-------|
| P-03 | grid/list toggle not persisted | no `Settings::save` call site |
| P-11 | no context menu | not implemented |
| P-12 | no menu items | follows P-11 |
| P-13 | Prefix tools unreachable | arms exist, no producer |
| P-14 | desktop shortcut unreachable | arm exists, no producer (#89) |
| P-15 | delete: empty arms, no producer, no dialog | not implemented |
| P-20 | file pickers | empty arms, no producer |
| P-24 | cover import / find cover | empty arms; no cover fetching at all |
| P-25 | Steam genre auto-categorise | follows P-60 |
| P-31 | save-triggered cover fetch | no producer; follows P-60 |
| P-33 | guide homepage buttons disabled | `button::link` with no `on_press`; test name over-claims |
| P-37 | runner removal: no confirm, stale list | pre-deletion snapshot at `view/runners.rs:1001` |
| P-57 | Locate dialog; page left permanently disabled | `easy_busy` stays true; no producer for Complete/Cancel; `easy_pending` never read |
| P-58 | install cover is not the vendor icon; Play action unproven | follows P-61 |
| P-60 | Steam cover search | not implemented |
| P-61 | exe-icon fallback / vendor icon | not implemented (no PE parser) |
| P-63 | cover download caps | not implemented |
| P-65 | Visit buttons emit nothing (at HEAD) | `Message::OpenUrl` absent; fixed in the working tree, uncommitted — and that change currently breaks the `doc` stage (`form.rs:48`) |
| P-66 | settings not persisted | no `Settings::save` call site |
| P-67 | Breeze palettes; system-scheme restore | deliberately not ported; "system" collapses to dark (#58) |
| P-69 | "every mutation" | 25 toast sites vs 39 `notify.emit`; Play closure untested |
| P-71 | desktop shortcuts | follows P-14 (#89) |

## 4. What this walk did not do

- It did not run the Flatpak. Every (B) procedure above is written to be
  executed as a script and **none of them has been executed by me**. P-64, P-70,
  P-71, P-72 and P-78 are Packaging's; I have classified them and named their
  evidence, not observed them.
- It did not re-run `bash scripts/verify.sh`. The twelve-stage green run at
  `177b135` is the coordinator's measurement, not mine.
- It did not test the two items whose evidence is a *mutation* rather than an
  assertion. I have one: the form-copy path in `state.rs:502-504` and `:519-521`
  writes six fields from `GameForm` into the stored game, and swapping
  `game.arguments`'s source field to `self.environment` survived the entire
  workspace suite with `Compiling` observed. That mutation is not filed against
  a P-item (the six copies are the plumbing under P-22/P-26/P-27/P-28/P-29/P-30)
  but it is the reason I would not call any of those six field copies "pinned".

**Measurement hygiene.** Every mutation above was made in `/tmp/t19`
(`git archive 9bcacf9 | tar -x`, its own `CARGO_TARGET_DIR=/tmp/t19-target`),
restored from a backup, and verified with
`for f in $(find crates -name '*.rs'); do git show 9bcacf9:$f | cmp -s - $f || echo "DIFFERS: $f"; done`
→ `(sweep clean)`. No tracked file in the working tree was modified by me.
