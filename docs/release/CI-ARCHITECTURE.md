# CI/CD architecture

How GameHandler's two GitHub Actions workflows are put together, and — the
part the YAML cannot say — why. This documents the implementation as it
exists in the tree: `.github/workflows/ci.yml`, `.github/workflows/release.yml`,
and the scripts they drive (`scripts/ci.sh`, `scripts/release-check.sh`,
`scripts/package-release.sh`, `scripts/package-flatpak.sh`,
`scripts/verify-release.sh`, `scripts/release-notes.sh`,
`scripts/publish-release.sh`). The release pipeline's step-by-step mechanics
are in `RELEASES.md` beside this file; the maintainer checklist is
`docs/RELEASING.md`.

Every claim about this repository cites a `file:line` read in this tree.
Claims about the hosted runners cite the external document they were checked
against. Where the tree cannot settle a question, the open point is stated
rather than smoothed over — that is the convention the audit docs run on.

## Why the tree has workflows at all

The packaging audit's PKG-02, in its own words: "Nothing in the repository
runs the verification chain automatically" (`docs/audit/PACKAGING.md`). The
17-stage chain in `scripts/verify.sh` was hand-invoked only, so a packaging
regression was caught when a human remembered to run a 2,700-line script, and
no artefact showed the chain ran against a given commit.

`scripts/ci.sh` is the caller the audit asked for — deliberately thin: it
picks the flag set, names the commit it ran against, tees the run to
`target/ci/<timestamp>-<sha>.log`, and exits with verify.sh's own status
(`ci.sh:13-18`, `:134-168`). It holds no copy of the stage list; a second
copy is exactly the drift verify.sh's `STAGES` array exists to stop
(`ci.sh:16-18`). The two workflows are the wiring PKG-02's fix text
described: "a CI workflow running at minimum verify.sh" — the entry point
was the deliverable, and wiring it is one line on a machine that has the
toolchain (`ci.sh:60-61`).

## The two workflows

| Workflow | Trigger | Jobs | Top-level permissions |
|---|---|---|---|
| CI (`ci.yml`) | push to `main`, every pull request (`ci.yml:18-21`) | `gate` on x86_64, `build-aarch64` on arm64 | `contents: read` (`ci.yml:23-24`) |
| Release (`release.yml`) | push of a `v*` tag, or `workflow_dispatch` with a required `tag` input (`release.yml:15-23`) | `validate` → `build` (both arches, matrix) → `publish` | `contents: read`; only `publish` takes `contents: write` (`release.yml:25-26`, `:122-123`) |

The release trigger's second door is a repair hatch, not a second pipeline:
every step resolves its ref as `inputs.tag || github.ref_name`
(`release.yml:44`, `:49`, `:67`, `:71`, `:125`, `:130`), so a dispatched
re-run checks out and builds the tag's commit, and the publish path is
idempotent — draft-or-reuse, `--clobber` upload, read-back gate
(`publish-release.sh:51-85`; the full sequence is in `RELEASES.md`).

## The runner fleet — native, and why

Two labels, verified against the GitHub-hosted runners reference
(docs.github.com/en/actions/reference/runners/github-hosted-runners — both
GA for public repositories, and this repo is public):

- `ubuntu-24.04` — x86_64
- `ubuntu-24.04-arm` — aarch64

Every job in both workflows runs on one of the two. There is no QEMU
anywhere in the pipeline and no cross toolchain. That is a decision, and
the reasoning is worth writing down because the obvious alternative —
build x86_64 natively, emulate or cross for aarch64 — loses on the three
axes that matter here:

1. **What is shipped is architecture-bearing artefacts.** The tarball's
   binary carries its arch in its ELF header, read back with `readelf`
   before packaging (`package-release.sh:73-79`); the bundle carries it in
   the imported OSTree ref, `app/com.goshapps.GameHandler/<arch>/stable`,
   read back after `build-bundle` (`package-flatpak.sh:118-124`) and again
   from a throwaway repo at publish time (`verify-release.sh:132-146`). On
   a native runner those reads confirm the runner's own output. Under
   emulation they would be confirming an emulator's output — the same
   assertion, about a different thing.
2. **The check that matters most executes the artefact.** The release
   build's smoke step installs the just-built bundle and runs
   `flatpak run com.goshapps.GameHandler --version` (`release.yml:103-108`).
   Under qemu-user that runs through an emulator, and Flatpak's sandbox —
   user namespaces, seccomp, ptrace — is exactly where user-mode emulation
   is least faithful. A green emulated smoke run proves the bundle boots
   under QEMU. This audit's recurring defect class is a check that passes
   without inspecting what it claims; an emulated boot is that class one
   layer down, at the platform.
3. **Cost.** A `cargo build --release` of a workspace that pins a libcosmic
   rev plus the iced stack (`crates/app/Cargo.toml:40`) under qemu-user is
   measured in hours. A cross toolchain needs an aarch64 sysroot carrying
   the same `-dev` packages the apt step installs (`release.yml:73-80`)
   plus linker and cargo configuration the tree does not otherwise need —
   and flatpak-builder would still need an aarch64 runtime and SDK, and
   would still execute target binaries mid-build. The native runner
   replaces all of that with one word in `runs-on`.

The reason aarch64 needs a *build* at all is in ci.yml's own header:
"libcosmic is the kind of dependency surface where aarch64-specific build
failures hide" (`ci.yml:9-11`). A green x86_64 gate says nothing about arm;
`build-aarch64` compiles natively and greps the ELF header for `AArch64`
(`ci.yml:76-82`).

## The gate

`gate` (`ci.yml:34-57`) runs on `ubuntu-24.04`. It installs the libcosmic
build dependencies plus `desktop-file-utils` and `appstream`
(`ci.yml:39-46`), caches the cargo directories under the `x86_64-gate` key
(`ci.yml:48-50`), prints the resolved toolchain (`ci.yml:53-54`), and runs
`scripts/ci.sh` — which is `scripts/verify.sh --skip-flatpak` plus
record-keeping (`ci.sh:143`).

The toolchain is `rust-toolchain.toml`'s pin: channel `1.98.1` with the
`clippy` and `rustfmt` components (`rust-toolchain.toml:11-13`). The first
cargo call resolves it through the runner's rustup; the gate logs the
resolution so the compiler version is in the log, not assumed. 1.98.1 is
also what the Flatpak's `org.freedesktop.Sdk.Extension.rust-stable//25.08`
ships (DECISIONS D-10; `Cargo.toml:20-23`), so the gate, the release
build's host compile, and the in-sandbox flatpak-builder compile all run
the same compiler. `rust-version = "1.93"` (`Cargo.toml:23`) is libcosmic's
floor, not the toolchain.

What each stage of the gate does on a clean hosted runner — one with no
`build-flatpak/` tree and no flatpak toolchain — is:

| Stage | On the gate runner |
|---|---|
| `build`, `fmt`, `clippy`, `doc`, `test`, `cli` | run — `cargo fmt --check`, `clippy -D warnings` at the workspace root (D-08), rustdoc intra-doc links, the suite in both feature configurations with `DISPLAY`/`WAYLAND_DISPLAY` unset, and the headless `--list`/`--launch`/`--version` driver (`verify.sh:190-196`) |
| `oracle-freshness` | runs — regenerates the fixtures into a temp dir with plain `python3` (the generators import `gamehandler.{models,settings,runners}`, all stdlib) and diffs against the checked-in copy (`verify.sh:1285-1358`) |
| `python-tests` | runs — `python3 -m unittest discover -s tests`; the suite is built for no-PySide6: `test_discovery.py` asserts `find_spec('PySide6') is None` and the QML smoke test guards its import |
| `cargo-lock`, `advisories`, `plan-counts` | run — `cargo metadata --offline --locked`, the RustSec ledger comparison (SEC-10), and PLAN.md's tables against their rows (`verify.sh:1400-1455`) |
| `cargo-sources` | runs — every `source = "git+…"` in `Cargo.lock` covered by `cargo-sources.json`, with the count required non-zero (`verify.sh:1560-1607`) |
| `cargo-sources-fresh` | **the swing stage — see below.** Runs only if an interpreter with `aiohttp` exists for the vendored generator (`verify.sh:1625-1667`, `find_generator_python` at `:1495-1505`) |
| `flatpak-build`, `smoke-test` | SKIP, *requested* — that is what `--skip-flatpak` asks for (`verify.sh:2612-2624`; the flag sets `SKIP_SMOKE` too, `:350`) |
| `desktop-metainfo` | SKIP, *requested* — the stage validates the copies installed into `build-flatpak/files`, which a clean runner does not have, and is skipped whole before its source-validation fallback (`verify.sh:2626-2634`) |
| `flatpak-contents` | its manifest half runs — every install line in the gamehandler module covered, every required install declared — then SKIP, *requested*, on the installed-copy half, because the absent tree is the flag's own consequence (`verify.sh:1988-2017`, `:2636-2639`) |

The exit contract is verify.sh's (D-31; `verify.sh:104-117`, `:2685-2751`):
`0` every stage passed or was skipped *because the caller asked*; `1` a
stage failed; `2` usage error; `3` the run was **incomplete** — a stage
skipped for a missing prerequisite, which is not a pass. ci.sh forwards the
status and prints which case occurred (`ci.sh:171-182`). A red `3` on the
gate means "a toolchain is missing", not "a defect" — the distinction the
code exists to make (`verify.sh:2696-2702`).

**`cargo-sources-fresh` is the one stage whose prerequisite the gate does
not provision.** The generator is vendored — `build-aux/flatpak/
flatpak-cargo-generator.py`, pinned upstream commit `f03a673`, provenance in
`build-aux/flatpak/VENDORED.md` — so `find_cargo_generator` always finds it
(`verify.sh:1475-1491`). What the stage then needs is an interpreter that
can `import aiohttp` (and, at generator run time, `tomlkit`; `PyYAML` is
optional — the PEP 723 block at the top of the vendored file is the
contract). The runner image's `python3` has neither (`aiohttp` appears
nowhere in the ubuntu-24.04 image readme's package list, and PEP 668 blocks
system pip installs), and the gate's apt step installs no Python packages
(`ci.yml:42-46`). The expected outcome on the stock image is therefore a
SKIP the caller did not ask for → exit `3` → a red gate that is honestly
reporting "this stage did not run". The designed-in fix is the lookup
itself: `find_generator_python` prefers `build-aux/flatpak/venv/bin/python`
(`verify.sh:1498-1500`), so a CI step that creates that venv with the PEP
723 set turns the stage from a skip into a real regeneration — which needs
the network, and the gate has it. Installing only `python3-aiohttp` is the
wrong half-fix: the generator would then start and fail on `import
tomlkit`, which the stage reports as FAIL, not SKIP
(`verify.sh:1670-1678`).

## The aarch64 compile check

`build-aarch64` (`ci.yml:59-82`) is deliberately not a packaging job. It
installs the same host deps minus the packaging tools (`ci.yml:64-70`),
runs `cargo build --locked` (`ci.yml:76-77`), then verifies the product's
architecture rather than the runner's: `readelf -h` on the binary must
report `Machine: …AArch64` (`ci.yml:79-82`). Asserting on the ELF rather
than on `uname -m` is the same discipline the packaging scripts use — the
claim is about the artefact, so the evidence comes from the artefact. It
builds the debug profile: the check exists to catch arch-specific compile
failures early, and `--release` time is spent in the release pipeline where
it produces something.

## The release pipeline — shape and load-bearing choices

Three jobs, `validate` → `build` (matrix) → `publish` (`release.yml:36-142`).
`RELEASES.md` walks the mechanics; the design decisions are:

- **Validation is a gate on version sources, before anything builds.**
  `release-check.sh` requires the tag, the workspace `Cargo.toml`
  (`[workspace.package] version` — `0.8.0`, `Cargo.toml:18`; the member
  manifests inherit it via `version.workspace = true`, so one edit bumps
  the workspace), the newest metainfo `<release version>`, and
  `gamehandler/__init__.py __version__` to agree, failing loudly on the
  first disagreement (`release-check.sh:25-72`). A release whose sources
  disagree ships a binary that reports one version under a tag that claims
  another; the script's own header calls that "cheap to stop here and
  expensive to explain later" (`release-check.sh:15-17`).
- **`fail-fast: false`** (`release.yml:57`): an aarch64 failure does not
  cancel the x86_64 leg — a partial result keeps both logs rather than
  scrubbing one.
- **`publish` needs both legs** (`release.yml:120`): a failed arch means no
  release at all, never a release with half the artefacts. The asset set is
  all-or-nothing by construction — see `RELEASES.md` for the five-file
  contract and the draft → upload `--clobber` → read-back → publish order
  (`publish-release.sh:32-85`).
- **Artefacts cross job boundaries as workflow artefacts, named per arch.**
  Each `build` leg uploads `release-<arch>` containing exactly that arch's
  `tar.gz` and `.flatpak`, with `if-no-files-found: error` — a leg that
  produced nothing fails loudly rather than forwarding an empty set — and
  `retention-days: 5`, because these are intermediates, not the product
  (`release.yml:110-117`). `publish` downloads `release-*` into `dist/`
  with `merge-multiple: true` (`release.yml:135-139`), then
  `verify-release.sh` re-verifies the merged set wholesale — reopening each
  tarball to read the ELF machine, re-importing each bundle to read the
  ref's arch — before `publish-release.sh` touches GitHub
  (`verify-release.sh:85-167`, `publish-release.sh:32-35`). Verification
  runs twice by design: per-arch inside the packaging scripts, and again
  over the assembled set at the boundary where a mix-up would ship.
- **The build leg smokes the artefact it just made.** Install the bundle,
  `flatpak info` the ref, run `--version` (`release.yml:103-108`) — the
  "the artefact boots" check, pointed at the build this job produced, on
  both arches. This is the assertion PKG-04 asked for.

The naming contract throughout is `gamehandler-v<VER>-linux-
<x86_64|aarch64>.{tar.gz,flatpak}` plus `SHA256SUMS`, which must verify and
name exactly those four (`verify-release.sh:157-167`).

## The AppArmor user-namespace step

Ubuntu 24.04 restricts unprivileged user namespaces through AppArmor — the
`kernel.apparmor_restrict_unprivileged_userns` sysctl — and bubblewrap,
which flatpak-builder uses for every build sandbox, needs them. On the
hosted image the restriction is on, so the release `build` job opens with

```
sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
```

(`release.yml:82-85`). Without it the flatpak-builder step fails in bwrap
before the manifest is read — a runner-environment prerequisite, kept as
its own named step so the log says which requirement was relaxed rather
than burying it inside the install. The CI `build-aarch64` job does not
need it because it never invokes flatpak-builder (`ci.yml:59-82`). For the
same reason the packaging step passes `--disable-rofiles-fuse`
(`package-flatpak.sh:100`): the build must not depend on FUSE being
available inside the runner's sandbox.

## The aarch64 Flatpak variant

The checked-in manifest is the x86_64 build: `org.winehq.Wine` base app,
`stable-25.08`, the `GL32`/`Compat.i386` inherited extensions,
`--allow=multiarch`, and a `dxvk-runtime` module shipping DXVK's x86
Windows DLLs (`build-aux/flatpak/com.goshapps.GameHandler.json:10-11`,
`:19`, `:29-32`, `:58-79`). None of the Wine side exists for aarch64:
flathub publishes no `runtime/org.winehq.Wine/aarch64/…` ref and no aarch64
`Compat.i386`/`GL32` (verified 2026-10 against `flatpak remote-info`), and
DXVK's tarballs are x86 DLLs that cannot run under an aarch64 Wine.
Freedesktop Platform, the Sdk, and the `rust-stable` extension do publish
aarch64 `25.08` — the base the application itself needs.

So `package-flatpak.sh` generates a variant manifest for `--arch aarch64`
rather than parametrising the checked-in one: a small Python edit drops
`base`/`base-version`/`inherit-extensions`, removes `--allow=multiarch`
from `finish-args`, and drops the `dxvk-runtime` module
(`package-flatpak.sh:77-95`). The `gamehandler` module — the application —
is architecture-neutral Rust on the freedesktop SDK and builds identically
(`package-flatpak.sh:14-16`). The consequence to know: **the two bundles
are not the same application inside different wrappers.** The aarch64
bundle has no Wine base, no 32-bit compat, and no bundled DXVK — it is
GameHandler without the Wine runtime the x86_64 bundle carries. That is
recorded in the script's header and here, not implied.

Two details are load-bearing in both manifests' path through the job: the
flathub remote is added at *user* scope before the build
(`release.yml:87-88`), matching the `--user` and
`--install-deps-from=flathub` flags the script passes flatpak-builder
(`package-flatpak.sh:97-109`); and the bundle is only named a release
artefact after the ref it produced is confirmed in the repo under the
requested arch (`package-flatpak.sh:118-124`).

## Caching

Every cargo job uses `Swatinem/rust-cache` with an explicit `key:` that
includes the architecture and the role (`ci.yml:48-50`, `:72-74`;
`release.yml:90-92`):

- `x86_64-gate` — the CI gate
- `aarch64-build` — the CI compile check
- `release-x86_64`, `release-aarch64` — the release matrix legs

The arch is in the key because GitHub's cache lookup matches on the key and
branch, not on the runner's architecture: a cache written on
`ubuntu-24.04-arm` is restorable on `ubuntu-24.04` when the keys agree, and
a `target/` dir restored across arches is not a cache hit, it is
contamination — worst case, an x86_64 binary sitting where an aarch64 job
expects its own output. The role is in the key so the gate's debug profile
and the release's `--release` profile never share a cache either.

## Action pinning

Every third-party and first-party action is pinned to a full-length commit
SHA with the resolved version in a trailing comment — checkout v7.0.1,
upload-artifact v7.0.1, download-artifact v8.0.1, rust-cache v2.9.2
(`ci.yml:37`, `:48`, `:62`, `:72`; `release.yml:42`, `:69`, `:90`, `:110`,
`:128`, `:135`). A tag is a ref the action's owner can move; a SHA is not.
The pin is the supply-chain half of the same discipline the scripts apply
to artefacts — identity by content, not by name — and the comment keeps the
pin reviewable: a SHA bump with no version comment is the diff to look at
twice.

## Permissions

- Top level: `contents: read` in both workflows (`ci.yml:23-24`,
  `release.yml:25-26`) — the default for every job.
- `publish` alone escalates to `contents: write` (`release.yml:122-123`),
  the narrowest scope that can create a release and upload assets;
  `validate` and `build` run read-only throughout.
- The `gh` CLI inside `publish-release.sh` authenticates as
  `GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}` (`release.yml:126`) — the
  workflow's own token, whose lifetime is the job's and whose permissions
  are what the job declares. No PAT, nothing to rotate.

## Concurrency

- CI: `group: ci-${{ github.ref }}`, `cancel-in-progress: true`
  (`ci.yml:26-28`). A new push to a PR or to `main` cancels the superseded
  run — the old result is about a commit nobody will read. Different refs
  never cancel each other.
- Release: `group: release-${{ inputs.tag || github.ref_name }}`,
  `cancel-in-progress: false` (`release.yml:29-31`). A second trigger of
  the same tag queues behind the first: a publish is never interrupted
  mid-upload, which is the half of the atomicity story the workflow itself
  owns (the other half — draft until the read-back passes — is
  `publish-release.sh`'s).

## What green does not mean

The limits, stated rather than implied:

1. **The gate does not build the Flatpak.** `flatpak-build`, `smoke-test`
   and `desktop-metainfo` are requested skips on a clean runner, and
   `flatpak-contents` exercises only its manifest half — the sections above
   name each skip. The complete chain is `scripts/ci.sh --full` on a
   machine with flatpak-builder and the freedesktop runtime (`ci.sh:21-23`);
   in CI, the release pipeline is where bundles are actually built,
   verified and smoked — per arch.
2. **`cargo-sources-fresh` is the gate's one unprovisioned prerequisite**
   (see "The gate"). On the stock image it exits the run `3` —
   INCOMPLETE — which is the designed signal and not a pass, but it is a
   red the apt step could have prevented.
3. **On a developer checkout that already has `build-flatpak/`**, the two
   tree-reading stages run — against *that* tree, which may predate the
   commit in the log (`ci.sh:35-43` says this in full). On the hosted
   runner the checkout is always clean, so the caveat applies to local gate
   runs, not to CI's.
4. **The release workflow does not re-run the verification chain.** It
   compiles `--release --locked`, packages, smokes and verifies the
   artefacts; it does not run `verify.sh`'s stage list. The assurance chain
   is: the gate keeps `main` green, `validate` ties the tag to the tree's
   own version sources, and the tag is expected to name a commit the gate
   saw. A tag pushed on an ungated commit ships whatever it names.
5. **Platform coverage is what it is:** Ubuntu 24.04, both arches. The
   Flatpak is the portable artefact; the tar.gz is built and ELF-verified
   but the host-library surface of other distributions is not exercised by
   either workflow.
