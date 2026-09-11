//! Runner management: Proton and Wine builds, archives, and launch options.
//!
//! A port of `gamehandler/runners.py` (1547 lines, 43 public names) plus the
//! half of `tests/test_security.py` that guards it. It is the largest module in
//! the project and the only one where part of the surface is a security
//! boundary rather than an application feature.
//!
//! # Layout
//!
//! `docs/migration/architecture.md` §1.1 splits the Python module into a
//! directory, and the split is by *trust boundary* rather than by file size —
//! `archive.rs` handles untrusted bytes and the rest does not:
//!
//! * [`archive`] — bounded extraction of an untrusted tarball, and validation
//!   of the staged tree. Landed first (PLAN.md R-3), because it is the only
//!   part where a defect is a vulnerability, and because it is the part a
//!   summary would get wrong.
//! * `families` — the runner catalogue and asset selection.
//! * the module root — the `Runner` trait, `WineRunner`, `ProtonRunner` and
//!   `RunnerManager`.
//! * `launch_opts` — the launch-option matrix, with `LaunchEnv` injected
//!   (DECISIONS D-27) so `which` and the anticheat probe stay testable.
//! * `proton` — `ProtonManager`: download, stage, rename into place.
//! * `launch` — process launch and early-exit reporting.
//! * `desktop` — `.desktop` shortcut generation.

pub mod archive;
pub mod families;
