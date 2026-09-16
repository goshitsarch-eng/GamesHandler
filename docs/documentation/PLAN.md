# Documentation plan

What each documentation file in this repository is for, and what this pass did
to it. Classifications: **KEEP** (left as-is), **REWRITE** (substantially
reworked), **MERGE** (folded into another file), **REMOVE** (deleted),
**CREATE** (new).

Source of truth for every verdict: the code under `crates/` (the shipped Rust
application), `gamehandler/` (the Python parity reference), the Flatpak
manifest, `scripts/`, and observed runtime behaviour of the built binaries.

## Audience-facing documentation

| File | Verdict | Purpose | Problems found | Source of truth |
|---|---|---|---|---|
| `README.md` | REWRITE | Project overview for new users and contributors | Version claims stale (Python tree reports 0.8.0, not 0.7.2); Flatpak permission section described `--filesystem=home` where the manifest grants `home:ro`; verify.sh stage list incomplete; Python build instructions outweighed the shipped app; no development-transparency notice | `crates/`, manifest, `flatpak run` probes, live CLI runs |
| `CONTRIBUTING.md` | CREATE | How to set up, test, and submit changes | Did not exist; contribution info was scattered in README | `scripts/verify.sh`, `scripts/ci.sh`, Cargo lints |
| `docs/documentation/APP-INVENTORY.md` | CREATE | Verified feature inventory | New — this pass's evidence base | `crates/app`, `crates/core`, runtime probes |
| `docs/documentation/AUDIT.md` | CREATE | Claim-by-claim fact-check record | New — this pass's evidence base | README + docs vs. source |
| `docs/documentation/PLAN.md` | CREATE | This file | New | — |

## Application metadata (already verified accurate)

| File | Verdict | Notes |
|---|---|---|
| `data/com.goshapps.GameHandler.metainfo.xml` | KEEP | AppStream data matches the shipped app: id, name, summary, 0.8.0 release notes describing the Rust/libcosmic port, keywords added under PKG-06. Still no `<screenshots>` — none exist to reference (PKG-06 residual). |
| `data/com.goshapps.GameHandler.desktop` | KEEP | Validated by `desktop-file-validate`; `Exec=gamehandler` matches the CLI. |

## Internal engineering records

These are process documents from the Qt→libcosmic port and the subsequent
audit. They are the project's working record, are internally consistent, and
are kept for contributors. They are not user documentation and are no longer
summarised in the README beyond a pointer.

| File | Verdict | Notes |
|---|---|---|
| `docs/migration/` (PLAN.md, REPORT.md, architecture.md, DECISIONS.md, packaging.md, ux.md, review-phase1.md, T19-PARITY-WALK.md, VERIFY-FINDINGS.md, oracle/) | KEEP | The port plan, parity checklist (P-01…P-78, B-01…B-08), decisions register and the frozen Python-behaviour oracle the test suite regenerates. `oracle/` fixtures are load-bearing: verify.sh's `oracle-freshness` stage compares them against the Python implementation. |
| `docs/audit/` (PLAN.md, REPORT.md, BUGS.md, COSMIC-UX.md, ARCHITECTURE.md, PACKAGING.md, PERFORMANCE.md, SECURITY.md, FEATURES.md, DECISIONS.md, BASELINE.md, advisories.json) | KEEP | The post-port audit record. `advisories.json` is load-bearing: verify.sh's `advisories` stage checks Cargo.lock against it. |

## Not created

| Candidate | Decision |
|---|---|
| `INSTALL.md` / `BUILDING.md` | Not created. One app, two build routes (cargo, flatpak-builder) — short enough to live in README and CONTRIBUTING without a third file. |
| `docs/user/` guide | Not created. The app is one window with six pages; README's usage section covers it. |
| Changelog file | Not created. Release history lives in the AppStream metainfo, where software centres actually read it. |
