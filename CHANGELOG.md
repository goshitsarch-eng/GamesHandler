# Changelog

## 0.9.0-alpha.1 — unreleased

- Add a Flutter desktop frontend and a coarse Rust C ABI with real backend tests.
- Retain compatible game/settings JSON and the original implementations.
- Add native launch, platform paths, portable artwork, Linux compatibility jobs,
  job progress/cancellation, menus, keyboard commands and window-size persistence.
- Add automatic native-library packaging, headless CLI, platform packaging and
  multi-OS CI/release definitions.
- Reject corrupt settings before overwrite; serialize new client writes.
- Match runner release assets to CPU architecture instead of picking ARM first.

This is an in-progress migration prerelease. Windows/macOS host QA, ARM host QA,
installed Flatpak/portal tests, Wine installer QA and hosted release verification
remain outstanding. See PLATFORM_SUPPORT.md and MIGRATION_AUDIT.md.
