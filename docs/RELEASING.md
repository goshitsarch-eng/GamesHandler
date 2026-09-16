# Cutting a release

A release is a `v*` tag push. GitHub Actions validates the tag, builds both
architectures, and publishes a GitHub Release carrying five assets — two
tarballs, two Flatpak bundles and a `SHA256SUMS`. The mechanics of what the
pipeline guarantees and in what order are in `docs/release/RELEASES.md`;
this file is the checklist.

## Before you tag — three version sources must agree

`scripts/release-check.sh` gates the whole pipeline on the tag saying the
same version as everything else. Update all three:

1. **Root `Cargo.toml`** — `[workspace.package] version = "X.Y.Z"`. One
   edit: both crates inherit it via `version.workspace = true`, so there is
   no per-crate bump.
2. **`gamehandler/__init__.py`** — `__version__ = "X.Y.Z"`. The Python tree
   is the retired parity reference, but its version is still checked.
3. **`data/com.goshapps.GameHandler.metainfo.xml`** — add a
   `<release version="X.Y.Z" date="YYYY-MM-DD">` element at the **top** of
   `<releases>`. The list is newest-first and the check reads the first
   element, so the new entry must precede the previous release. Its
   `<description>` is not boilerplate: it becomes the GitHub Release body
   verbatim (flattened to markdown — paragraphs stay paragraphs, `<ul>/<li>`
   become bullets, `<em>`/`<code>` become `*`/`` ` ``), so write it for the
   release page as much as for the app stores.

Commit the lot. Then sanity-check before pushing anything:

```
scripts/release-check.sh vX.Y.Z
```

It prints `ok`/`FAIL` per source and exits non-zero on any disagreement —
far cheaper to see it here than in a failed workflow run.

## Tag and push

```
git tag vX.Y.Z
git push origin vX.Y.Z
```

That push is the entire release act. The `Release` workflow
(`.github/workflows/release.yml`) takes it from there:

- **validate** — runs `release-check.sh` on the tag. A mismatch stops
  everything; nothing builds and nothing publishes.
- **build** — native `cargo build --release --locked` on `ubuntu-24.04`
  (x86_64) and `ubuntu-24.04-arm` (aarch64), then packaging per arch and a
  smoke stage that installs the Flatpak and runs `--version`.
- **publish** — verifies the complete artefact set (including reading each
  binary's ELF machine and each bundle's OSTree ref arch), regenerates
  `SHA256SUMS`, creates a **draft** release titled `GameHandler vX.Y.Z` with
  the metainfo notes as its body, uploads all five assets, reads the asset
  list back to confirm every one is present and non-empty, and only then
  flips the draft public and marks it latest.

When the run goes green the release is already live — there is no manual
publish step and nothing further to attach.

## Re-running a release — the escape hatch

Pushing the same tag again is neither necessary nor sufficient on its own
(the tag would need moving). Instead: **Actions → Release → Run workflow →
enter the existing tag** (e.g. `v0.8.0`). The dispatch input makes every
step check out that tag's commit and re-run the pipeline against it.

The publish step is idempotent, so this is the repair path:

- the release is found by tag — no duplicate is created;
- assets re-upload with `--clobber`, so a partial or interrupted upload is
  overwritten cleanly;
- the post-upload read-back re-verifies all five assets before the release
  is (re)published.

One caveat: an existing release's **title and body are not refreshed** on a
re-run — those are set at creation. If the notes need fixing after a draft
already exists, edit the draft or delete the release and re-run.

## When it fails

- **validate failed** → version disagreement. The log names each failing
  source (`Cargo.toml workspace version`, `metainfo newest <release>`,
  `gamehandler/__init__.py`). Fix, commit, move the tag
  (`git tag -f vX.Y.Z && git push -f origin vX.Y.Z`) or dispatch a re-run
  after re-tagging.
- **a build leg failed** → the other arch still completes (`fail-fast:
  false`), so both logs are there to read. Nothing is published.
- **publish failed before or during upload** → check the step log. A
  verification failure means no release exists; an asset-check failure
  means a **draft** was left behind — it is invisible publicly, so either
  fix and re-run via dispatch, or delete the draft and start clean.
