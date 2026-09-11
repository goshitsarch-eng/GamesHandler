# UX Migration: Qt 6 + Kirigami → libcosmic

Source of truth for UI parity. Every item in the current interface
(Python 3 + PySide6 + QML styled with Kirigami, at `cbaf7e6`) is inventoried
with `file:line`, mapped to a verified libcosmic API from the reference clone
at `/tmp/cosmos-spike/libcosmic`, and judged against COSMIC design conventions.

**How API names were verified:** by grepping the reference clone
(`src/widget/…`, `src/theme/…`, `src/app/…`, `examples/`). Anything not
confirmed this way is marked **UNSURE** — do not treat those names as final;
re-check against the pinned libcosmic revision at implementation time.

Notation used below: `[direct]` = libcosmic has a same-purpose widget;
`[adapt]` = achievable with a nearby widget plus small custom code;
`[custom]` = must be built; no direct equivalent exists.

---

## 1. Application shell (`gamehandler/qml/Main.qml`, `gamehandler/main.py`)

| # | Current item | Location | libcosmic mapping |
|---|--------------|----------|-------------------|
| S1 | App window 1180×760, min 420×480, title "GameHandler" | `Main.qml:8-15` | `[direct]` `cosmic::app::Settings` (`.size()`, `.min_size()` if available — verify) + `cosmic::app::Core`; title via `Application::title`. Pattern: `examples/application/src/main.rs`. |
| S2 | Left navigation drawer, non-modal, collapsible, titled "GameHandler" | `Main.qml:58-62` | `[direct]` `cosmic::widget::nav_bar` (`src/widget/nav_bar.rs:25`) driven by a `segmented_button::Model`. COSMIC HIG prefers the nav bar over a drawer for top-level pages. Collapsible behavior comes free with the nav bar. |
| S3–S8 | Six nav entries: Library (`applications-games`), Installers (`run-install`), Runners (`folder-download`), Plugins (`plugins`), About & Credits (`help-about`), Settings (`configure`); checkable, checked = current page | `Main.qml:64-107` | `[direct]` `nav_bar` model entries with `cosmic::widget::icon::from_name()` handles. **7th destination:** the GameForm is a pushed layer in Qt (S11), see F1 mapping. |
| S9 | Drawer footer `ToolButton` "Powered by Wine, Proton & DXVK" → credits page | `Main.qml:110-116` | `[adapt]` Nav bar has no footer slot; put an equivalent link-button at the bottom of the Credits page, or a `text_button` in the header bar area. Minor. |
| S10 | Page stack with clear + push on `showPage()` | `Main.qml:37-45` | `[direct]` `cosmic::app` nav-bar router: `nav_id` + `on_nav_select` pattern from `examples/nav-context/src/main.rs`. Page switching = model activation, no manual stack. |
| S11 | Game form pushed as overlay **layer** with injected `dismiss` callback | `Main.qml:47-54` | `[adapt]` No page-stack layers in libcosmic. Options: (a) a 7th nav-bar page shown conditionally; (b) `widget::dialog` modal; (c) `widget::context_drawer`. Recommendation: full-page dialog (form is 362 lines, needs the width). See §6 risk R6. |
| S12 | Plugins page auto-refresh on navigation | `Main.qml:43-44` | `[direct]` Trigger the refresh `Task` in the `on_nav_select` handler for the plugins entity. Trivial. |
| S13 | Passive toast notification (`showPassiveNotification`) | `Main.qml:150-152` | `[direct]` `widget::toaster::{toaster, Toast, Toasts}` (`src/widget/toaster/mod.rs:20,122`); app holds `Toasts<Message>`, pushes on backend events. |
| S14 | Notification **with action button** ("Play" after install) | `Main.qml:154-161` | `[direct]` `Toast::action(label, message)` (`toaster/mod.rs:132`) + `Toast::duration`. Verified. |
| S15 | `locateDialog`: exe-picker shown when easy-install can't find the exe (token + start folder) | `Main.qml:182-188`, `bridge.py:886-895` | `[adapt]` `cosmic::dialog::file_chooser::open::Dialog` (pattern in `examples/open-dialog/src/main.rs:154-163`) with `FileFilter`; async `open_file().await`. Token/start-folder flow becomes an app-state enum. Name filters verified (`FileFilter` import at `open-dialog:8`). |
| S16 | Hide window on launch / re-show on early-exit error | `Main.qml:170-179`, `bridge.py:470-483` | `[adapt]` Window visibility via `cosmic::app::Core` window management (`Task` on `window::Id`; exact helper UNSURE — verify `core.main_window_id()` / minimize helpers). Behavior, not widget. |
| S17 | Window/app icon + Breeze fallback icon theme | `main.py:92-101` | `[direct]` Icon theme via `Settings::default_icon_theme("Pop")` (`nav-context:44`); app icon from the existing `data/icons` install. Platform icons resolve through freedesktop names — same names as Kirigami uses. |
| S18 | Headless CLI (`--list`, `--launch`, `--version`) bypasses GUI | `main.py:118-131` | `[direct]` Keep verbatim in the Rust binary (clap or manual args). No UI impact, listed so shortcut parity (§5) stays honest. |

---

## 2. Library page (`gamehandler/qml/LibraryPage.qml`)

| # | Current item | Location | libcosmic mapping |
|---|--------------|----------|-------------------|
| L1 | Page title "Library"; header action "Add Game" (`list-add` icon) | `LibraryPage.qml:8-24` | `[direct]` Nav-bar pages carry their own `header_bar`; page action → `widget::header_bar` button or nav-bar context button. Verify exact header-bar button API at build time (module `widget/header_bar.rs` exists). |
| L2 | Search field, placeholder "Search games…", max-width `gridUnit*22` | `LibraryPage.qml:30-36` | `[direct]` `widget::search_input` (`src/widget/text_input/input.rs:91`). Width via `Length::Fixed`. |
| L3 | Category filter `ComboBox` + "Filter by category" tooltip | `LibraryPage.qml:38-54` | `[direct]` `widget::dropdown` (`src/widget/dropdown/mod.rs:23`). Tooltip → `widget::tooltip::tooltip` (`mod.rs:335`). |
| L4 | Sort `ComboBox` (Name / Recently played / Recently added) | `LibraryPage.qml:56-65`, `bridge.py:67-71` | `[direct]` Same `widget::dropdown`. Options come from the backend enum unchanged. |
| L5 | Grid/list view toggle `ToolButton`, icon swaps (`view-list-icons` ↔ `view-list-details`), checkable | `LibraryPage.qml:67-74` | `[adapt]` `widget::button::IconButton` with dynamic `icon::from_name`; **no tooltip API on icon button verified** — check, else wrap in `tooltip`. Or `segmented_button` horizontal two-option control (`segmented_button/horizontal.rs:24`). |
| L6 | Empty-library `PlaceholderMessage` + 3 centered buttons (Add first game / Easy install / Download a runner) | `LibraryPage.qml:82-110` | `[adapt]` No placeholder widget in libcosmic; compose `column![icon, text::title3, text::body, buttons]`. Icon: `widget::icon` large. |
| L7 | No-results `PlaceholderMessage` + "Clear filters" | `LibraryPage.qml:112-128` | `[adapt]` Same composition as L6. |
| L8 | `GridView`, fixed 200×300 cells, margins `largeSpacing`, hover tint, **double-click = play**, **right-click = menu** | `LibraryPage.qml:132-141,149-166` | `[custom]` libcosmic `widget::grid::{Grid, grid}` (`mod.rs:225`) is a manual row builder (`push`, `insert_row` — `grid/widget.rs:73,109`), **not** a virtualized data view. Card sizing/wrapping must be hand-rolled (fixed columns or `flex_row`). Double-click and right-click gestures: **UNSURE** — iced has mouse-area events but double-click detection and per-card context-menu anchoring need verification. See risk R1. |
| L9 | Game card: rounded-rect tile (radius 14), cover art, bold centered title (elided), small dim subtitle, Play button + "More" tool button | `LibraryPage.qml:143-218` | `[adapt]` `widget::card` (module exists, `mod.rs:122-124`) or a styled `container` + `column![image, text, row![buttons]]`. Cover → `widget::image` (re-exported iced image, `mod.rs:69`). Bold/center/small text → `widget::text` styling + `text::body/caption`. Elision: **UNSURE** whether iced `Text` exposes Qt-style `ElideRight` — iced has ellipsis support; verify exact API. |
| L10 | `ListView` rows: `ItemDelegate` h=`gridUnit*3.4`, compact cover, name + `subtitle · lastPlayed`, Play + More | `LibraryPage.qml:223-287` | `[direct]` `widget::list::{list_column, ListColumn, ListButton}` (`list/list_column.rs:29,110`) — the idiomatic COSMIC list. Row content = same cover + labels + buttons. |
| L11 | Per-game context menu: Play / Edit / Find cover art / ─ / Winecfg / Winetricks / Open prefix folder / ─ / Create desktop shortcut / Remove from library; prefix items disabled for Linux games | `LibraryPage.qml:298-343` | `[adapt]` `widget::context_menu::{ContextMenu, context_menu}` (`mod.rs:148`) or `menu::menu_bar` trees. Per-card anchoring + right-click trigger is the unverified part (same as L8). Disabled-when-Linux → `MenuTree`/`MenuItem::enabled` (`menu_tree.rs:257` — `enabled()` verified). Icons per item → `MenuItem::icon` (`menu_tree.rs:236` — verified). See risk R2. |
| L12 | Remove-game `PromptDialog` (Cancel + destructive Remove) | `LibraryPage.qml:345-360` | `[direct]` `widget::dialog` (`dialog.rs:5`): `.title()`, `.body()`, `.primary_action()`, `.secondary_action()`. Verified. |
| L13 | Tooltips on icon-only buttons ("More actions", "Toggle list view", sort, filter) | `LibraryPage.qml:41-42,61-62,71-72,211-212,282` | `[direct]` `widget::tooltip::tooltip` (`mod.rs:335`). Every icon-only control keeps one. |
| L14 | Row data contract (`gameId, name, subtitle, coverUrl, coverIsIcon, initials, accent, isLinux, lastPlayed`) | `bridge.py:304-322` | Backend contract, unchanged. `coverIsIcon` letterbox rule (CoverArt.qml:60) must be preserved in the image widget (`ContentFit` equivalent — verify name). |

---

## 3. Game form (`gamehandler/qml/GameFormPage.qml`, 362 lines — largest page)

Container mapping: `[adapt]` `widget::settings::{section, item}` (`settings/section.rs:10`, `settings/item.rs:17,82`) — the six `Kirigami.Separator` section headers (Game / Library / Compatibility tool / Launch options / Compatibility / Advanced) become `settings::section().title(…)` blocks, which is exactly what they are for under COSMIC HIG. Form rows: label + control = `settings::item::builder(title).control(widget)` (`item.rs:82,107`) or `.flex_control` for wide rows.

| # | Current item | Location | libcosmic mapping |
|---|--------------|----------|-------------------|
| F1 | Page itself: title switches Add/Edit; Cancel + Add/Save actions; **Save enabled only when name non-empty** | `GameFormPage.qml:18,25-37` | `[adapt]` If form = dialog (S11): `dialog` primary/secondary actions; dynamic enablement = rebuild dialog element from state each `view()` — free in Elm architecture. Validation message "A game needs a name" (`bridge.py:412`) → toast or `text_input::error()` (`input.rs:314` — verified). |
| F2 | Name `TextField` | `GameFormPage.qml:81-85` | `[direct]` `widget::text_input` (`input.rs:55`) with `.on_input`, or labeled variant `.label()` (`input.rs:295`). |
| F3 | Type `ComboBox`: Windows (Wine/Proton) / Linux native | `GameFormPage.qml:87-92` | `[direct]` `widget::dropdown`. |
| F4 | Executable row: text field + Browse `ToolButton` (`document-open`) opening exe `FileDialog` with `*.exe` filter + auto-fill name from basename | `GameFormPage.qml:94-107,334-347` | `[adapt]` `text_input` + icon button → `file_chooser::open::Dialog` with `FileFilter` (verified pattern, `open-dialog`). Basename auto-fill is app logic, unchanged. |
| F5 | Launch arguments field | `GameFormPage.qml:109-113` | `[direct]` `text_input`. |
| F6 | Working directory field | `GameFormPage.qml:115-119` | `[direct]` `text_input`. |
| F7 | Category **editable** ComboBox (defaults "Uncategorized") | `GameFormPage.qml:126-132` | `[adapt]` `widget::dropdown` is **not** editable (verified struct: `Dropdown<S: AsRef<str>…>` over a fixed list, `dropdown/widget.rs:31`). Closest faithful alternative: `widget::combo_box` (re-exported iced ComboBox, `mod.rs:60`) which **is** editable + filterable, or dropdown + separate "new category" text row. Prefer iced `combo_box`. |
| F8 | Cover row: 2×3-gridUnit preview, "No cover yet" placeholder, "Find cover" button + tooltip, custom-cover browse button | `GameFormPage.qml:134-165` | `[adapt]` `widget::image` fixed size + `text::caption` placeholder + two buttons. `coverUrlFor`/`importCustomCover` backend Slots unchanged (`bridge.py:589-609`). Image-picker dialog: image `FileFilter` (`*.png/.jpg/.jpeg/.webp` — `GameFormPage.qml:352`). |
| F9 | Runner `ComboBox`, disabled for Linux | `GameFormPage.qml:172-183` | `[direct]` `dropdown` + `on_press_maybe(None)`-style disabling (dropdown enablement API: verify; buttons/inputs all have `*_maybe` variants). |
| F10 | Wine prefix field, placeholder "Leave empty for an isolated prefix per game", disabled for Linux | `GameFormPage.qml:185-191` | `[direct]` `text_input` with `.helper_text()` (`input.rs:301` — verified) replacing the QML placeholder. |
| F11–F18 | 8 Launch-option switches w/ description text: MangoHud, GameMode, Prefer SDL, Wayland (exp.), HDR (exp.), Esync, Fsync, Gamescope | `GameFormPage.qml:198-249` | `[direct]` `widget::toggler` (`toggler.rs:17`) `.on_toggle` (verified) inside `settings::item::builder(title).toggler(…)` (`item.rs:144` — verified). Description → item `.description()` (verify name; `Item` builder has description support per `button` analogy — **UNSURE** exact fn name on `Item`, check `item.rs:82-144`). |
| F19–F25 | 7 Compatibility switches + Desktop-size field (enabled iff virtual-desktop on): DXVK, VKD3D, NVAPI/DLSS, FSR, BattlEye, EAC, Virtual desktop + `1920x1080` field | `GameFormPage.qml:256-312` | `[direct]` Same toggler pattern; conditional enablement via `on_toggle_maybe` (`toggler.rs:141` — verified). Desktop-size → `text_input`, consider `spin_button`? No — free text `WxH`, keep `text_input`. |
| F26 | Additional application field + placeholder | `GameFormPage.qml:319-324` | `[direct]` `text_input` + helper text. |
| F27 | Environment variables field, placeholder "KEY=value pairs. Overrides the toggles above" | `GameFormPage.qml:325-330` | `[direct]` `text_input` + helper text. |
| F28 | Linux-native mode disables runner/prefix/compat switches (`isLinux`, `enabled: !isLinux`) | `GameFormPage.qml:16,175,188,218…` | `[direct]` Disablement is state-driven in `view()`; COSMIC HIG also allows hiding inapplicable sections — recommend **hiding** the Compatibility-tool/Compatibility sections for Linux games rather than showing them greyed (cleaner; behavior change to confirm). |
| F29 | Post-save auto cover fetch when empty | `bridge.py:444-445` | Backend behavior, unchanged. Progress/toast feedback via toaster. |

Toggle count check: form has 15 switches (8 + 7); both map to the same two APIs.

---

## 4. Installers page (`gamehandler/qml/InstallersPage.qml`)

| # | Current item | Location | libcosmic mapping |
|---|--------------|----------|-------------------|
| I1 | Header toolbar: search field + category ComboBox + spacer | `InstallersPage.qml:12-28` | `[direct]` Page header content: `search_input` + `dropdown`. Header-toolbar composition in libcosmic = `header_bar` content / page header row — verify exact slot API at build time. |
| I2 | `InlineMessage` explainer (vendor installer → isolated prefix → library) | `InstallersPage.qml:33-36` | `[adapt]` No inline-message widget found in the clone. Alternatives: `widget::warning` (module exists, `mod.rs:349` — verify constructors), or `text::body` in a styled container. **UNSURE** which is idiomatic; check `warning.rs`. |
| I3 | `ProgressBar` (visible iff busy && progress ≥ 0) | `InstallersPage.qml:38-43` | `[direct]` `progress_bar::determinate_linear(progress)` (`progress_bar/mod.rs:22`). Indeterminate phase → `indeterminate_linear()` (`mod.rs:12`). Same `backend.busy/progress` contract. |
| I4 | "Runner for new installs" ComboBox + two-line explainer | `InstallersPage.qml:48-73` | `[direct]` `dropdown` in a `settings::item` with description. Live re-sync on `runnersChanged` (`:58-64`) = Elm `view()` re-render, free. |
| I5 | "Catalog" `Heading` level 3 | `InstallersPage.qml:75-78` | `[direct]` `widget::text::title3` (see §6 typography). |
| I6 | Empty `PlaceholderMessage` (no matching installers) | `InstallersPage.qml:80-85` | `[adapt]` Same composed placeholder as L6. |
| I7 | Installer cards: name `Heading` 4 + category **Chip**, small subtitle, Install `Button` + tooltip | `InstallersPage.qml:87-134` | `[custom]` Card composition as L9; **Chip has NO libcosmic equivalent** (no chip/badge/pill found anywhere under `src/widget/`). Build a pill: `container` with rounded style + `text::caption`. Reused for R5 category/release chips. See risk R3. |

---

## 5. Runners page (`gamehandler/qml/RunnersPage.qml`)

| # | Current item | Location | libcosmic mapping |
|---|--------------|----------|-------------------|
| R1 | Header action "Refresh" (`view-refresh`), disabled while loading | `RunnersPage.qml:21-28` | `[direct]` Header-bar button; disabled state via `on_press_maybe(Option)` pattern. |
| R2 | ProgressBar (same busy contract) | `RunnersPage.qml:33-38` | `[direct]` `determinate_linear`, as I3. |
| R3 | "Installed" section: cards with status icon (`emblem-checked` / `data-warning`), bold name + small detail, delete `ToolButton` + tooltip (removable only) | `RunnersPage.qml:40-96` | `[direct]` `list_column` rows or cards; status via `icon::from_name`; delete via icon button + tooltip. |
| R4 | "No downloaded runners yet" hint label | `RunnersPage.qml:90-96` | `[direct]` `text::body`/`caption`. |
| R5 | Family `ComboBox` + auto description/maintainer label + "Project:" `UrlButton` | `RunnersPage.qml:114-149` | `[direct]` `dropdown` + `text::body` + link button: `widget::button::LinkButton` (`mod.rs:114`) or `button::link` — opens URL via `open` crate / portal. `UrlButton` semantics = open-in-browser; keep. |
| R6 | "Available versions" + tri-state status label (loading / error / empty) | `RunnersPage.qml:151-166` | `[adapt]` `text::body` driven by `releasesStatus` ("idle/loading/ready/error:…", `bridge.py:689-692`); error text preserved verbatim. Loading additionally gets `indeterminate_linear`. |
| R7 | Release cards: bold tag, `family · asset · MB` detail, "Installed" **Chip**, Install button (`download` icon, disabled while busy) | `RunnersPage.qml:168-205` | `[custom]` Card as L9; Installed pill = same custom chip as I7. |
| R8 | Guide cards: title H4 + "Visit project" `UrlButton`, kind·maintainer line, advice paragraph | `RunnersPage.qml:220-253` | `[direct]` Card + `text::title4/heading`, link button, `text::body`. Straightforward. |
| R9 | Remove-runner `PromptDialog` (Cancel + destructive Remove, explains System Wine fallback) | `RunnersPage.qml:256-271` | `[direct]` `widget::dialog`, as L12. Subtitle text preserved verbatim. |
| R10 | `Separator`s between sections | `RunnersPage.qml:98,207` | `[direct]` `widget::divider::horizontal::default()` (`mod.rs:155-162` — verified). |

---

## 6. Plugins page (`gamehandler/qml/PluginsPage.qml`)

| # | Current item | Location | libcosmic mapping |
|---|--------------|----------|-------------------|
| P1 | "Host plugins" heading + intro paragraph (Flatpak vs host-package-manager variants) | `PluginsPage.qml:15-25`, `bridge.py:986-1000` | `[direct]` `text::title3` + `text::body`. Both intro strings preserved; backend `pluginsIntro` unchanged. |
| P2 | Plugin cards: bold name, subtitle (description + used_for / install command / unavailable reason), tri-state button Installed (disabled) / Install (enabled) / Unavailable (disabled) | `PluginsPage.qml:27-56`, `bridge.py:951-979` | `[direct]` Card/list rows + `widget::button` with three states. Install runs async with toast feedback (`bridge.py:1006-1031`) — same contract. This page is the strongest `settings::section` candidate if rows are reframed as items; either is HIG-fine. |

---

## 7. About & Credits (`gamehandler/qml/CreditsPage.qml`)

| # | Current item | Location | libcosmic mapping |
|---|--------------|----------|-------------------|
| C1 | Title "About & Credits"; H3 "Standing on other people's work"; "Made by Gosh."; version label | `CreditsPage.qml:10-27` | `[adapt]` libcosmic ships `widget::about::about` (`about.rs:98`, `mod.rs:365`) with `developers/artists/designers/documenters/translators/links` builders (`about.rs:55-98`) — but it renders a **dialog**, not a page, and the current page carries far more (credit sections, why-all-in-one essays, license footer). Recommendation: keep a custom About page with the same content; optionally surface `about::about` from an app-menu "About" entry. |
| C2 | Acknowledgement paragraph | `CreditsPage.qml:29-34` | `[direct]` `text::body`, wrapped. |
| C3 | Credit sections: separator, H3 title, summary, per-entry cards (bold label, role + License line, "Visit" UrlButton, hidden when no URL) | `CreditsPage.qml:36-94` | `[direct]` Sections + cards + link buttons; conditional Visit = `if url.is_empty()` skip in `view()`. Backend `creditSections` contract unchanged. |
| C4 | "Why one app…" entries (H4 + body) | `CreditsPage.qml:98-124` | `[direct]` `text::title4/heading` + `body`. |
| C5 | License footer + "GameHandler on GitHub" UrlButton | `CreditsPage.qml:128-140` | `[direct]` `text::caption` + link button. |
| C6 | Version string (`backend.appVersion`, currently 0.7.2) | `CreditsPage.qml:25-27` | `[direct]` Build-time `CARGO_PKG_VERSION` or backend-provided; same display. |

---

## 8. Settings page (`gamehandler/qml/SettingsPage.qml`)

Natural `settings::section` page — the closest 1:1 mapping in the whole app.

| # | Current item | Location | libcosmic mapping |
|---|--------------|----------|-------------------|
| T1 | Appearance section: color-scheme ComboBox (Match system / Light / Dark) | `SettingsPage.qml:39-51` | `[adapt]` `dropdown` in `settings::item`. BUT theming semantics change — see §6 "dark by default". |
| T2 | Library layout ComboBox (Grid / List) | `SettingsPage.qml:53-70` | `[direct]` `dropdown`; live-sync on `settingsChanged` is free in Elm. |
| T3 | Default runner ComboBox | `SettingsPage.qml:77-95` | `[direct]` `dropdown`. |
| T4 | 13 "…by default" switches (mangohud, gamemode, prefer_sdl, esync, fsync, dxvk, vkd3d, nvapi, fsr, battleye, eac, gamescope, virtual_desktop), some with " — subtitle" appended | `SettingsPage.qml:12-26,98-110` | `[direct]` `settings::item::builder(label).toggler(state, on_toggle)` (`item.rs:144` — verified) or `.description()` + `.toggler()`. NOTE: wayland/hdr have **no** defaults (bridge.py:79-82) — preserved as form-only. `virtual_desktop` default exists though the form's desktop-size default ("1920x1080") is form-level. |
| T5 | "Hide window when launching" switch + description | `SettingsPage.qml:120-125` | `[direct]` Toggler item, same pattern. Backend `closeOnLaunch` unchanged. |
| T6 | Shortcut cheat-sheet (4 labeled rows) | `SettingsPage.qml:127-135` | `[adapt]` Static `settings::section` rows with `text::caption` showing the keybind text; values must mirror §5 exactly. Consider rendering actual `KeyBind` strings from one shared table so docs and behavior can't drift. |

---

## 9. Shared visuals (`CoverArt.qml`, `theme.py`)

| # | Current item | Location | libcosmic mapping |
|---|--------------|----------|-------------------|
| V1 | Cover tile: async cached image (512px, smooth), portrait crop for store art, **letterboxed** exe icons on gradient plate, rounded corners (10, 6 compact), initials when no art | `CoverArt.qml:39-66` | `[custom]` `widget::image` + `clip` container for rounding (verify iced clipping API); async loading via `Task` + image `Handle::from_bytes`. Crop-vs-letterbox (`PreserveAspectCrop` vs `PreserveAspectFit`, `:60`) maps to iced `ContentFit::{Cover, Contain}` (names to verify). See risk R1. |
| V2 | 8 gradient plate pairs, stable per-game `accent` index | `CoverArt.qml:19-26`, `theme.py:30-38`, `bridge.py:315` | `[custom]` Gradient rendering in iced (gradient background support: verify; else pre-render plates or flat colors from the same hex table — keep the hexes so shades survive, per the comment at `theme.py:27-28`). |
| V3 | Breeze dark/light palettes + `#3daee9` accent; system passthrough | `theme.py:14-74` | `[adapt]` COSMIC theming is `cosmic::theme` (system-driven). Forcing Breeze-exact colors fights the toolkit — see §6. Accent `#3daee9` can be approximated via a custom `Theme`/`palette` override (verify extension points). |
| V4 | Density roles: `Units.smallSpacing/largeSpacing`, `gridUnit`, `iconSizes.smallMedium`, `Theme.smallFont` | throughout QML | `[direct]` `cosmic::theme::spacing().space_xxs … space_xxl` (usage: `examples/application/src/main.rs:367` — `.space_s`). Exact numeric mapping TBD at build time against `theme::Spacing`; rule: smallSpacing→`space_s`, largeSpacing→`space_m`, gridUnit multiples→`space_l/xl`. Small/dim labels → `text::caption`. |
| V5 | Typography: `Heading` levels 3/4, bold labels, small dim secondary text | throughout QML | `[direct]` `text::title1..title4`, `text::heading`, `text::body`, `text::caption` (`text.rs:28-119` — all verified). Map: H3→`title3`, H4→`title4`, section labels→`heading`/`caption_heading`, secondary→`caption`, body copy→`body`. **UNSURE**: whether `titleN` sizes visually match Kirigami H3/H4 — eyeball in review. |

---

## 10. Keyboard shortcuts and accessibility (§5 in brief — full section follows)

Current shortcuts (`Main.qml:121-143`, cheat-sheet `SettingsPage.qml:132-135`):

| Shortcut | Action | libcosmic equivalent |
|----------|--------|----------------------|
| Ctrl+N | Add game (blank form) | `menu::KeyBind{ modifiers:[Ctrl], key:Character("n") }` (`menu/key_bind.rs:23-47` — struct + `matches()` verified) wired in the `HashMap<KeyBind, Action>` menu infrastructure (`menu_tree.rs:288,365`), plus a Toast/button path. If shortcuts must work outside menus, verify `cosmic::app` keyboard subscription support — **UNSURE**. |
| Ctrl+F | Library + focus search | Same `KeyBind` mechanism; focus via `text_input` id + focus `Task` (`Id` support: `input.rs:308`). |
| Ctrl+, | Settings page | Same mechanism. Note: `,` is layout-sensitive; `KeyBind::matches` has non-Latin physical-key fallback (`key_bind.rs:47-70`) — good. |
| Ctrl+Q | Quit | Same mechanism; reuse `Application` quit Task. |

**Accessibility obligations for the port:**

- **Labels:** every icon-only control currently has a tooltip (L13); keep `tooltip` on all of them — tooltips are the accessible-name source. Form inputs get real labels via `settings::item::builder(title)` (F-section) or `text_input::label()` (`input.rs:295`).
- **Focus order:** Qt tab order was implicit; in iced, order = widget tree order — keep header controls before content, Play before More, Cancel before destructive actions. Verify `Tab` moves through `text_input`s (iced handles natively; confirm in testing).
- **Screen readers:** iced accessibility hooks exist (`Button::description/label` — `button/widget.rs:273-289` verified). Set descriptions on icon-only buttons and toggles. **Testing with Orca is a QA task** — flag it, don't assume.
- **i18n:** all user strings are currently English literals in QML + `bridge.py` notify texts. libcosmic convention is `i18n-embed` + Fluent (`src/localize.rs:3-13` — `fluent_language_loader!`, `fl!` macro verified). Port must route every literal in this inventory through `fl!` with an `i18n/` folder. Count: 100+ strings (every label, tooltip, toast, dialog body, placeholder above). No translation exists today, so English Fluent defaults = no user-visible change, but the plumbing must land in this migration or retrofitting doubles the work.

---

## 11. Parity risks (UI-specific)

- **R1 — Library grid virtualization + gestures (L8, V1).** `GridView` is virtualized with hover tint, double-click-to-play, right-click menus. libcosmic `grid` is a manual layout builder, not a data view; large libraries need manual windowing or a `scrollable` column of cards. Double-click/right-click handling is unverified. *Resolution:* custom card-grid component with hover state, single-click select + Play button as the primary path (already exists on every card), click-count timing for double-click if iced exposes it; verify `mouse_area`/`context_menu` anchoring early — spike this first, it is the highest-risk UI work.
- **R2 — Per-card context menus (L11).** Qt anchors the menu to the card/tool button. libcosmic `context_menu`/`menu` trees verified to exist, but per-item anchor APIs were not confirmed. *Resolution:* if anchoring is weak, the "More" button opens a `popover`/`dropdown`-style menu; keep all 10 items + separators + Linux-disablement regardless of container.
- **R3 — No Chip/badge widget (I7, R7).** Category pills and "Installed" pills must be custom-built (rounded container + caption). *Resolution:* one shared `pill()` helper; trivial but must exist or two pages lose information density.
- **R4 — Dark-by-default vs COSMIC system theming (T1, V3).** README advertises "Dark mode by default… both first-class". COSMIC apps follow the system theme; hard-forcing dark fights HIG and `cosmic::theme`. *Resolution:* default to system theme, keep explicit Light/Dark overrides (current "Match system" already exists — `SettingsPage.qml:45`). This is a deliberate, documentable behavior change: first-run appearance follows the desktop instead of forced dark. Confirm with maintainer.
- **R5 — Editable category combo (F7).** `dropdown` takes a fixed list; free-text categories need iced `combo_box`. *Resolution:* use `combo_box`; verify its styling matches COSMIC inputs acceptably.
- **R6 — Game form as dialog vs page (S11, F1).** A 362-line form with 30+ controls in a modal dialog is cramped; as a 7th nav page it pollutes navigation. *Resolution:* recommend a full-size modal `dialog` (or nav-bar page hidden from the bar — verify support); either way Save-enablement, section layout, and Linux-disablement must be re-verified control by control against §3.
- **R7 — File-dialog UX deltas (F4, F8, S15).** Portal file pickers look/behave differently from Qt `FileDialog` (no custom sidebar, different name-filter UI), are async, and Flatpak-portal behavior differs from host. Start-folder seeding for the easy-install fallback (S15) may not be honored by the portal. *Resolution:* accept portal UX; keep filters; if start-folder is ignored, show the expected path in the dialog-adjacent toast (backend already emits explanatory text).
- **R8 — Cover-plate gradients (V2).** If iced backgrounds can't do gradients cheaply per-card, plates become flat colors from the same palette. *Resolution:* acceptable fallback, keep hex table; visual-review sign-off.
- **R9 — Typography/spacing eyeball risk (V4, V5).** `title3` vs Kirigami H3, `space_m` vs `largeSpacing` will not be pixel-identical. *Resolution:* one review pass over every page comparing screenshots; no functional impact.
- **R10 — Toast action parity (S14).** The post-install "Play" toast button maps to `Toast::action` (verified), but auto-dismiss timing vs Qt's "long" duration needs a sensible `Duration` choice. Minor.

---

## Appendix A — Backend contract the UI binds to (unchanged by this doc, listed for the implementer)

`gamehandler/bridge.py`: `Backend` QObject — properties `games, categories, sortOptions, sortMode, viewMode, categoryFilter, searchText, colorScheme, defaultRunner, defaultToggles, closeOnLaunch, runnerChoices, installedRunners, runnerFamilies, runnerGuide, releases, releasesStatus, busy, progress, installers, installerSearch, installerCategory, installerCategories, plugins, pluginsIntro, creditSections, whyAllInOne, acknowledgement, aboutText, appVersion, appId, librarySize, formCategories`; Slots `quit, saveGame, getGame, newGameTemplate, removeGame, playGame, runPrefixTool, openPrefix, createShortcut, fetchCover, fetchCoverForForm, importCustomCover, urlToLocalFile, coverUrlFor, fetchReleases, installRelease, uninstallRunner, installEasy, completeEasyInstall, cancelEasyInstall, installPlugin, refreshPlugins, setDefaultToggle`; signals `notify, gameInstalled, requestHide, requestShow, gamesChanged, categoriesChanged, settingsChanged, runnersChanged, releasesChanged, installersChanged, pluginsChanged, busyChanged, progressChanged, coverFetched, easyInstallNeedsExe`. Threading model (`bridge.py:147-166`): worker threads + queued `_dispatch` → in Rust becomes `Task`/`subscription` + `tokio::spawn` — backend-team concern, noted so toast/progress ordering expectations survive.

## Appendix B — Files read for this inventory

`gamehandler/qml/Main.qml`, `LibraryPage.qml`, `GameFormPage.qml`,
`InstallersPage.qml`, `RunnersPage.qml`, `PluginsPage.qml`, `CreditsPage.qml`,
`SettingsPage.qml`, `CoverArt.qml`, `gamehandler/theme.py`,
`gamehandler/main.py`, `gamehandler/bridge.py`, `README.md`,
`git log --oneline -20` (HEAD `cbaf7e6`, branch `cosmic-migration`;
Qt/Kirigami rewrite confirmed at `3c4b735` — no GTK remains).
