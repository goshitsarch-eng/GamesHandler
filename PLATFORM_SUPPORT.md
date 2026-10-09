# Platform support and verification

Version 0.9.0-alpha.1 is a migration prerelease. The complete production rewrite
has **not** been validated. Source support does not establish runtime support.

| Target | Implementation | Evidence in this session | Remaining gate |
|---|---|---|---|
| Linux x86_64 | Flutter + Rust; native launch, Wine/Proton jobs, prefixes, covers, helpers | Release build and packaged ABI/CLI smoke test; real ABI/widget/desktop tests; visible CRUD, cover fetch, themes, launch and window persistence | Real Wine/vendor wizard QA |
| Linux ARM64 | Native-host build path and CI job; architecture-filtered runners | Portable source only | Native ARM host CI and runtime QA |
| Windows x64 | Flutter shell, native process launch, platform directories, covers, .lnk shortcuts, ZIP/NSIS | Full Rust backend cross-build and Clippy; shared core tests compile for Windows target | Windows-host bridge/build/installer/runtime tests |
| macOS ARM64/Intel | Flutter shell/native menus, process/.app launch, platform directories, covers, .app shortcuts | Shared core and tests compile for ARM macOS target | Native host builds, signing, menus/file access and runtime QA |
| Flatpak x86_64 | GNOME 49 + Wine BaseApp; GTK portals, bundled Rust/CLI, DXVK and signature verifier | Verified SDK/BaseApp download, actual sandbox build and installation; runtime dependency defect found and corrected | Rebuilt installed runtime, portal and Wine QA |

Linux compatibility switches and prefix tools are capability-specific. Windows
runs Windows executables directly. macOS uses native applications; selecting a
Windows game produces an explicit unsupported-workflow error. Vendor catalogue
links on Windows/macOS lead to official native installers; they do not pretend
to run a Linux Wine wizard. Optional helper install instructions show/copy a
validated command rather than silently executing a host package manager.

Linux preserves existing XDG locations. Windows uses
`%APPDATA%/com.goshapps/GameHandler`; macOS uses
`~/Library/Application Support/com.goshapps.GameHandler`. Explicit
`GAMEHANDLER_CONFIG_HOME` and `GAMEHANDLER_DATA_HOME` overrides remain available.
Back up an existing library before prerelease testing. No schema migration is
performed. Native executables, mapped shares and arguments should be verified
on each target, including spaces and non-ASCII account paths.

Windows artifacts are unsigned; macOS artifacts are ad hoc signed and not
notarized. Native-host CI is configured; execution results will be recorded after the
feature branch run completes. No GitHub release has been published or inspected. Do not call this a
verified cross-platform release until the matrix and manual checklist pass.
