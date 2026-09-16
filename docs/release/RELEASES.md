# Releases

How a pushed `v*` tag becomes a GitHub Release, what that release carries,
and which guarantees the pipeline makes — and at which step. This documents
the implementation as it exists in the tree: `.github/workflows/release.yml`,
`scripts/release-check.sh`, `scripts/package-release.sh`,
`scripts/package-flatpak.sh`, `scripts/verify-release.sh`,
`scripts/release-notes.sh` and `scripts/publish-release.sh`. The
maintainer-facing checklist is `docs/RELEASING.md`; the CI context is
`docs/release/CI-ARCHITECTURE.md`.

Every claim below cites a `file:line` read in this tree.

## Trigger

Two doors into the same pipeline (`release.yml:15-23`):

- **push of a `v*` tag** — the normal path; or
- **`workflow_dispatch` with a required `tag` input** — the escape hatch for
  re-running a release without pushing anything. Every step resolves its ref
  as `inputs.tag || github.ref_name` (`release.yml:44`, `:49`, `:67`, `:70`,
  `:125`, `:130`), so a dispatched re-run checks out and builds **the tag's
  commit**, not the branch the dispatch was fired from.

A `concurrency` group keyed on the tag — `release-${{ inputs.tag ||
github.ref_name }}` — with `cancel-in-progress: false` (`release.yml:29-31`)
means a second trigger of the same tag queues behind the first rather than
cancelling it: a publish is never interrupted mid-upload by a retrigger.

## Pipeline shape

Three jobs (`release.yml:36-142`):

| Job | Runs on | Does |
|---|---|---|
| `validate` | `ubuntu-24.04` | checks out the tag and runs `scripts/release-check.sh "$TAG"`; exports `version=${TAG#v}` for the other jobs (`release.yml:37-53`) |
| `build` | `ubuntu-24.04` for x86_64, `ubuntu-24.04-arm` for aarch64 | `cargo build --release --locked`, then `package-release.sh` (tar.gz) and `package-flatpak.sh` (.flatpak) for that arch, then a smoke stage that installs the bundle and runs `--version`; uploads the two artefacts as a workflow artefact (`release-{arch}`, 5-day retention — an intermediate, not the product) (`release.yml:54-117`) |
| `publish` | `ubuntu-24.04` | downloads both `release-*` artefact sets into `dist/`, installs flatpak/ostree (needed by verify-release.sh's bundle check), runs `scripts/publish-release.sh "$TAG" dist` (`release.yml:119-142`) |

`build` carries `fail-fast: false` (`release.yml:57`): an aarch64 failure
does not cancel the x86_64 leg, so a partial result is visible rather than
scrubbed.

## Version agreement — the validate gate

`release-check.sh` refuses to let a tag ship a version the tree disagrees
with. For tag `vX.Y.Z` it asserts, in order (`release-check.sh:25-67`):

1. the tag looks like `vX.Y.Z` — `v[0-9]*` (`:25-28`);
2. the workspace version — read from `[package]`/`[workspace.package]` in
   `crates/app/Cargo.toml` first, falling back to the root `Cargo.toml`
   (`:31-44`). In practice the member manifests carry `version.workspace =
   true` (no quoted version to match), so the root `[workspace.package]
   version` is the value checked — one edit bumps the whole workspace;
3. the **newest** `<release version>` in
   `data/com.goshapps.GameHandler.metainfo.xml` — `root.find("./releases/
   release")` returns the first `<release>` element, so the metainfo's
   newest-first ordering is load-bearing (`:47-53`);
4. `__version__` in `gamehandler/__init__.py` — the Python parity tree is
   retired from shipping but remains a version source of truth (`:46`,
   `:67`).

Any disagreement exits 1 with each disagreeing source named
(`release-check.sh:69-72`). The check is cheap to run by hand —
`scripts/release-check.sh v0.8.1` — before the tag is pushed.

## The asset contract

A release is complete when it carries **exactly five assets**
(`verify-release.sh:8-21`, `publish-release.sh:58-63`):

```
gamehandler-v<VERSION>-linux-x86_64.tar.gz
gamehandler-v<VERSION>-linux-aarch64.tar.gz
gamehandler-v<VERSION>-linux-x86_64.flatpak
gamehandler-v<VERSION>-linux-aarch64.flatpak
SHA256SUMS
```

`SHA256SUMS` must verify against the four artefacts and name nothing else —
the list is compared sorted against the expected four
(`verify-release.sh:157-167`).

Verification is deeper than presence (`verify-release.sh:85-154`):

- each tarball is opened; its top level must be exactly the one staging
  directory; the staged layout is asserted (`bin/gamehandler`, `install.sh`,
  `LICENSE`, the `.desktop`, the metainfo, the hicolor SVG icon, the
  Microsoft root CA PEM); and `bin/gamehandler`'s ELF machine is read with
  `readelf` and compared to the arch in the filename;
- each `.flatpak` is imported into a throwaway OSTree repo and the imported
  ref's arch is read back — `app/com.goshapps.GameHandler/<arch>/stable` —
  which is the bundle's own metadata, not the filename's claim;
- every file must exist and be non-zero bytes.

The same contract is enforced twice: once per arch inside the packaging
scripts (`package-release.sh` reads the ELF machine before archiving;
`package-flatpak.sh` verifies the ref arch), and again wholesale by
`verify-release.sh` in the publish job before anything is uploaded.

## Publish order — a release nobody can partially see

`publish-release.sh` runs its steps in an order chosen so that no public
state is ever half-populated (`publish-release.sh:8-21`, `:32-85`):

1. **`verify-release.sh` must pass on `dist/` first** — the complete set,
   right arches (`:32-35`). Failure here means nothing is created at all.
2. **`SHA256SUMS` is (re)generated in place** against exactly the four files
   being shipped (`:38-42`), then the directory is **re-verified** (`:43-46`)
   — the checksum file describes what is uploaded, not what some earlier
   stage happened to produce.
3. **The release body is rendered** by `release-notes.sh` into
   `dist/.release-notes.md` (`:48-49`).
4. **A draft is created, or an existing release reused** — `gh release view
   "$TAG"` decides (`:51-66`). New releases are created with
   `gh release create "$TAG" --draft --title "GameHandler $TAG"
   --notes-file …` — draft, so nothing is public while assets land. When the
   release already exists *published*, it is first re-drafted (`--draft=true`)
   so the asset-replacement window below is never visible publicly; its title
   and body are refreshed via `gh release edit` in the same step.
5. **All five assets upload with `--clobber`** (`:68-73`).
6. **Read-back gate** — `gh release view "$TAG" --json assets` is queried and
   each of the five expected names must be present with `size > 0`
   (`:76-92`). Any miss prints `FAIL asset <name> missing or empty` and the
   script exits 1 **leaving the release unpublished** (`:93`).
7. **Publish** — `gh release edit "$TAG" --draft=false` plus `--latest` only
   when the tag is the newest `v*` on the remote (`:95-105`), so re-running an
   older tag cannot steal the "latest" pointer from a newer release.

That is the atomicity story: the release is public only in one state — all
five assets attached and verified. First runs stay drafts until the read-back
passes; repairs to a live release re-draft it before touching its assets, so
there is no window where a user sees a partial asset set.

## Idempotency — what a rerun does

The whole path is safe to re-run against the same tag, which is what makes
`workflow_dispatch` a repair tool rather than a footgun:

- **No duplicate releases.** `gh release create` is keyed on the tag and a
  GitHub release is 1:1 with its tag; the script's own `gh release view`
  check (`publish-release.sh:51`) finds the existing release first anyway,
  so a second run takes the update path by construction.
- **Assets are re-uploaded `--clobber`** (`publish-release.sh:68`): an
  interrupted upload is repaired in place — re-uploading overwrites
  same-named assets rather than 409ing or duplicating them.
- **A live release is re-drafted first** (`publish-release.sh:56-58`): the
  `--clobber` delete+upload sequence is a brief window where the release has
  a partial asset set, so a published release is pulled back to draft for the
  duration. Nothing visible is ever inconsistent — at worst the release is
  briefly a draft.
- **Title and notes are refreshed on every run** — `gh release edit
  --title --notes-file` runs on the reuse path (`publish-release.sh:62`),
  so correcting the metainfo notes and re-dispatching the workflow repairs
  the release body in place.
- **Verification runs again end-to-end**: both the pre-upload
  `verify-release.sh` and the post-upload read-back execute on every run, so
  a rerun of an already-published release still proves the set is whole
  after re-upload.

## Permissions model

- **Top level: `contents: read`** (`release.yml:25-26`) — the default for
  every job in the workflow.
- **Only `publish` escalates: `contents: write`** (`release.yml:122-123`) —
  the narrowest scope that can create a release and upload assets. `validate`
  and `build` run read-only throughout.
- **`GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}`** (`release.yml:126`) — the `gh`
  CLI inside `publish-release.sh` authenticates with the workflow's own
  token. There is no PAT, no separate credential, and nothing to rotate;
  the token's lifetime is the job's lifetime and its permissions are
  exactly what the job declares.

## Release title and notes

- **Title:** `GameHandler v0.8.0` — literal `GameHandler $TAG`, tag prefix
  included (`publish-release.sh:54`).
- **Body:** `release-notes.sh` extracts the `<description>` of the matching
  `<release version="X.Y.Z">` from
  `data/com.goshapps.GameHandler.metainfo.xml` and flattens the XML to plain
  markdown — `<p>` stays paragraphs, `<ul>/<li>` becomes a bullet list,
  `<em>`/`<code>` become `*`/`` ` `` spans (`release-notes.sh:38-58`). The
  curated notes — the same text app stores render — win over GitHub's
  auto-generated commit list by design (`release-notes.sh:8-11`).
- **Fallback:** if no `<release>` matches, the body is the one line
  `GameHandler X.Y.Z` (`release-notes.sh:34-36`) so `--notes-file` always
  has content. In the pipeline this is unreachable — `release-check.sh` has
  already required the newest metainfo `<release>` to equal the tag — but it
  keeps the script usable standalone.

## Failure map

| Where it fails | Outcome |
|---|---|
| `release-check.sh` in `validate` | nothing builds, nothing publishes; the log names each disagreeing version source |
| a `build` leg | `fail-fast: false` keeps the other arch's log; `publish` never runs (it `needs` both) |
| `verify-release.sh` inside publish | exit 1 before any `gh` call — no release is created |
| upload partially fails | the read-back gate catches the missing/empty asset and exits 1 — the release stays a draft, invisible publicly |
| anything after draft creation | rerun via `workflow_dispatch` on the same tag: assets re-upload `--clobber`, the read-back re-verifies, then the draft publishes |
