# Release packaging

How a tagged GameHandler release becomes the files under `dist/`, what each
artifact contains and why, and what is checked before any of it is called a
release. This documents the pipeline as it exists — the scripts under
`scripts/`, the manifest at `build-aux/flatpak/com.goshapps.GameHandler.json`,
and the workflow at `.github/workflows/release.yml` — with `file:line`
citations for the parts that are easy to describe wrongly from memory.

The release set is built per architecture (`x86_64`, `aarch64`) by two
packaging scripts, verified as a set by a third, and published by a fourth.
Nothing here is a proposal; where a check does not cover something, that gap
is stated rather than smoothed over.

## The artifact set

For a version `<VER>` the shippable set is exactly five files:

| File | Produced by |
|---|---|
| `gamehandler-v<VER>-linux-x86_64.tar.gz` | `scripts/package-release.sh` |
| `gamehandler-v<VER>-linux-aarch64.tar.gz` | `scripts/package-release.sh` |
| `gamehandler-v<VER>-linux-x86_64.flatpak` | `scripts/package-flatpak.sh` |
| `gamehandler-v<VER>-linux-aarch64.flatpak` | `scripts/package-flatpak.sh` |
| `SHA256SUMS` | `scripts/publish-release.sh:38-42` |

The version in every filename comes from the Cargo workspace, parsed with an
awk helper (`version_of`, e.g. `package-release.sh:36-45`) rather than `cargo
metadata` or jq so the scripts need no JSON tooling on the host. The helper
prefers a literal `[package] version` in `crates/app/Cargo.toml` and falls
back to `[workspace.package] version` in the root `Cargo.toml`
(`package-release.sh:64-67`). `crates/app` declares `version.workspace =
true` — which the awk deliberately does not match — so the workspace value
(`0.8.0` at `Cargo.toml:18`) is what lands in the names.

Naming is `gamehandler-v<VER>-linux-<arch>.<ext>` plus `SHA256SUMS`. The
`dist/gamehandler-0.8.0.flatpak` still in the tree predates the convention:
it was produced by `build-aux/flatpak/build.sh`, the arch-agnostic local
entry point, which still writes `gamehandler-<VER>.flatpak`
(`build-aux/flatpak/build.sh:60-65`). Release artifacts go through the two
`scripts/package-*.sh` entry points.

## The tarball: a staged prefix layout, not a build dump

`scripts/package-release.sh` requires `target/release/gamehandler` to already
exist — the caller builds it (`cargo build --release --locked`), so the
script works on a tree built by hand, by CI, or by flatpak-builder
(`package-release.sh:13-15, 70-71`). It stages a directory named
`gamehandler-v<VER>-linux-<arch>/` under `mktemp -d`, fills it with `install
-D`, tars it with `tar -czf`, and removes the stage on exit
(`:81-83, 127`). Only the tarball remains.

Before anything is staged, the binary's ELF machine is read with `readelf -h`
and compared against `--arch` (`:58-79`): `x86_64` expects `Advanced Micro
Devices X86-64`, `aarch64` expects `AArch64`. An x86_64 binary packaged under
an aarch64 name fails here rather than on a user's machine.

### What the tarball contains

| Staged member | Source in the repo | Why it ships |
|---|---|---|
| `bin/gamehandler` (0755) | `target/release/gamehandler` | The application itself (`package-release.sh:85`). |
| `share/applications/com.goshapps.GameHandler.desktop` (0644) | `data/com.goshapps.GameHandler.desktop` | The launcher entry. Its `Exec=gamehandler` resolves against `PATH`, so once `install.sh` has put the binary under `<prefix>/bin` the entry launches a working app (`:86`; `data/com.goshapps.GameHandler.desktop:5`). |
| `share/metainfo/com.goshapps.GameHandler.metainfo.xml` (0644) | `data/com.goshapps.GameHandler.metainfo.xml` | The AppStream record software centres render — description, keywords, screenshots, release history (`:87`). |
| `share/icons/hicolor/{scalable,64x64,128x128,256x256,512x512}/apps/com.goshapps.GameHandler.{svg,png}` (0644) | `data/icons/hicolor/…` | One SVG plus PNGs at the four sizes consumers that cannot rasterize SVG expect — the fix recorded under PKG-05 in `docs/audit/PACKAGING.md` (`:88-95`). |
| `share/gamehandler/microsoft-identity-verification-root-ca-2020.pem` (0644) | `data/microsoft-identity-verification-root-ca-2020.pem` | The pinned Authenticode trust root — see below (`:96-97`). |
| `LICENSE` (0644) | `LICENSE` | The GPL-3.0-or-later text (`:98`). |
| `install.sh` (0755) | generated inline | Copies the staged files into a prefix — see below (`:102-124`). |

### The `.pem` is an Authenticode root, not a TLS pin

`microsoft-identity-verification-root-ca-2020.pem` is the Microsoft Identity
Verification Root Certificate Authority 2020 — the subject is readable inside
the PEM itself. The app uses it as a pinned **code-signing** trust anchor for
the Windows installers its easy-installer recipes download, not for TLS:
when a recipe sets `microsoft_trust_root`, `verify_installer_authenticity`
splices `-CAfile <pem> -TSA-CAfile <pem>` into `osslsigncode verify` ahead of
`-in` (`crates/core/src/installers/signature.rs:173-186`). Exactly one recipe
sets the flag — Ubisoft Connect, whose signer chains to this 2020 root that a
host trust store does not necessarily carry
(`crates/core/src/installers/mod.rs:365-368`). The other nine recipes verify
against the verifier's default store and never read the file.

At runtime the root is located by `authenticode_root_path`
(`signature.rs:38-60`), which tries, in order:

1. `GAMEHANDLER_AUTHENTICODE_ROOT`, an environment override — the mechanism
   that makes the check testable and that reaches a `.pem` anywhere else.
2. `<CARGO_MANIFEST_DIR>/../../data/<name>` — a compile-time path baked in by
   `env!`, which resolves in a source or build tree and on nothing else.
3. `/app/share/gamehandler/<name>` — the Flatpak install location.

The tarball stages the file at `share/gamehandler/` — the same relative path
the Flatpak populates under `/app` — so an installed prefix mirrors the
shipped layout and the file is where the layout says it belongs. What should
be said plainly: the resolver does **not** search `$PREFIX/share/gamehandler`
on a non-Flatpak install. On a `~/.local` install the staged copy is reached
only via `GAMEHANDLER_AUTHENTICODE_ROOT`; without the override, the one
recipe that pins this root fails with `AuthenticodeRootUnavailable` rather
than silently verifying against the wrong anchor (`signature.rs:59`, and the
test at `:1097-1103` which names that arm deliberately uncovered). That is
the current behaviour, documented because the staging implies broader
coverage than the resolver provides.

### `install.sh`

A generated POSIX `sh` script (`package-release.sh:102-123`), not packaged
logic: `install.sh [PREFIX]` copies every staged file verbatim into `PREFIX`
(default `$HOME/.local`) with `install -D`. The binary lands at
`$PREFIX/bin/gamehandler`, the desktop file at `$PREFIX/share/applications/`,
and so on; `LICENSE` is installed at `$PREFIX/LICENSE`. It registers nothing
beyond what a desktop file needs — no icon-cache rebuild, no database update,
no uninstaller.

## The Flatpak

Both architectures are built by `scripts/package-flatpak.sh`, which runs the
same `flatpak-builder` invocation against different manifests
(`package-flatpak.sh:97-114`):

```
flatpak-builder --user --force-clean --disable-rofiles-fuse \
    --arch=<arch> --default-branch=stable --repo=flatpak-repo \
    [--install-deps-from=flathub | --disable-download] \
    build-flatpak <manifest>
```

`--install-deps-from=flathub` is the default — it resolves and installs the
runtime and SDK — and `GAMEHANDLER_BUILD_OFFLINE=1` swaps it for
`--disable-download`, the same flag shape `build-aux/flatpak/build.sh:42-53`
honours: a build told not to use the network should not silently use it.

After the build, `ostree refs --repo=flatpak-repo` must contain exactly
`app/com.goshapps.GameHandler/<arch>/stable` (`:118-124`). The ref name
itself records the architecture, so a bundle cannot be produced under a name
that lies about its arch. Only then does `flatpak build-bundle --arch=<arch>`
write `dist/gamehandler-v<VER>-linux-<arch>.flatpak` (`:127-132`).

### x86_64: the checked-in manifest

`build-aux/flatpak/com.goshapps.GameHandler.json` is the x86_64 build:

- `org.freedesktop.Platform` 25.08 runtime and `org.freedesktop.Sdk`, with
  the `org.freedesktop.Sdk.Extension.rust-stable` SDK extension
  (`manifest:4-9`).
- `base: org.winehq.Wine`, `base-version: stable-25.08` — a Wine install
  inside the sandbox — plus `inherit-extensions` for
  `org.freedesktop.Platform.GL32` and `Compat.i386`, and `--allow=multiarch`
  in the finish-args (`:10-11, 19, 29-32`): the 32-bit story 32-bit Windows
  games need.
- Three modules (`:39-119`):
  - `osslsigncode` 2.14 (`cmake-ninja`, archive pinned by sha256) — the
    Authenticode verifier `verify_installer_authenticity` execs; its licence
    lands in `share/licenses/com.goshapps.GameHandler/` (`:41-57`).
  - `dxvk-runtime` (`simple`) — the `x32`/`x64` DLL trees out of
    `dxvk-3.0.2.tar.gz` installed to `/app/share/gamehandler/dxvk`, the path
    `DXVK_ROOT` points at (`crates/core/src/runners/mod.rs:81`), plus the
    DXVK licence (`:58-79`).
  - `gamehandler` (`simple`) — `cargo --offline fetch --locked` then
    `cargo --offline build --locked --release` with
    `CARGO_HOME=/run/build/gamehandler/cargo` and `CARGO_NET_OFFLINE=true`,
    dependencies satisfied entirely by the committed
    `build-aux/flatpak/cargo-sources.json` (generated by the vendored
    `flatpak-cargo-generator.py`; provenance in
    `build-aux/flatpak/VENDORED.md`). It then `install -Dm…`s the same set
    the tarball stages — binary, desktop file, metainfo, five icons, the
    `.pem` to `/app/share/gamehandler/` (resolver candidate 3 above) — with
    `LICENSE` under `share/licenses/com.goshapps.GameHandler/` (`:80-118`).

The module's source is the repository itself as `type: dir`, with a `skip`
list covering `.git`, `.flatpak-builder`, `build-flatpak`, `flatpak-repo`,
`dist` and `target` (`:106-115`) — version-control metadata and prior build
output never enter the build context.

### aarch64: a generated manifest

Flathub publishes `org.winehq.Wine` only as the app ref
`app/org.winehq.Wine/x86_64/stable-25.08`; there is no aarch64 build of it
anywhere on flathub (the check is named in the script header,
`package-flatpak.sh:9-14`). Freedesktop Platform, the SDK and the rust-stable
extension all publish aarch64/25.08, so they carry over unchanged. For
aarch64 the script generates `target/com.goshapps.GameHandler.aarch64.json`
from the checked-in manifest (`:77-95`), dropping four things:

- `base` / `base-version` — the Wine base app that does not exist for aarch64.
- `inherit-extensions` — GL32 and Compat.i386 are x86-only extensions.
- `--allow=multiarch` — the finish-arg that only means something with the
  i386 compat layer.
- the `dxvk-runtime` module — DXVK's tarballs ship x86 Windows DLLs, which
  cannot run under an aarch64 Wine regardless.

The `gamehandler` module — the application — is architecture-neutral Rust
and builds identically on both arches. The deliberate consequence is that
the aarch64 Flatpak ships **without** a bundled Wine: the app manages its own
Proton/Wine downloads from their maintainers' release pages rather than
relying on the base app, so runner provisioning on aarch64 is the user's own
concern, the same as on any non-Flatpak install. The generated manifest is a
build product under `target/`; the checked-in manifest stays the x86_64 one.

## Checksums

`SHA256SUMS` is not written by the packaging scripts. `publish-release.sh`
regenerates it over exactly the four artifacts being shipped
(`publish-release.sh:38-42`) — after `verify-release.sh` has already passed
on the directory and before the draft release is created — and then re-runs
verification so the sums provably describe the uploaded bytes (`:43-46`).
`verify-release.sh:161-167` enforces the exact set: the file must verify with
`sha256sum --check` **and** name exactly the four artifacts, no more and no
fewer.

## Verification: what is checked, and at which layer

There are two layers, and it matters which is which.

**At packaging time**, inside each script:

- `package-release.sh` `readelf`s `target/release/gamehandler` and refuses a
  binary whose ELF machine disagrees with `--arch`.
- `package-flatpak.sh` requires the arch-named ref
  `app/com.goshapps.GameHandler/<arch>/stable` to exist in the build repo
  before `build-bundle` runs.

**At release time**, `scripts/verify-release.sh DIR` checks the assembled
directory (`verify-release.sh:85-168`):

- The four artifact filenames for the version exist and are non-zero.
- Each tarball is a readable gzip tar whose single top-level entry is the
  staging directory name; the required members are present — `bin/gamehandler`,
  `install.sh`, `LICENSE`, the desktop file, the metainfo, the SVG icon and
  the `.pem` (`:101-110`; the four PNG sizes are staged but not individually
  asserted); and the extracted `bin/gamehandler`'s ELF machine matches the
  arch in the filename (`:112-119`).
- Each `.flatpak` is imported into a throwaway `archive-z2` OSTree repo with
  `flatpak build-import-bundle`, and `ostree refs` on that repo must show
  `app/com.goshapps.GameHandler/<arch>/stable` — the bundle's own metadata,
  not the filename's claim (`:132-146`). If `flatpak`/`ostree` are not
  installed this is a FAIL, not a skip (`:128-131`).
- `SHA256SUMS` verifies against the four files and lists nothing else.

In CI there is a third, smaller layer: `release.yml`'s smoke step installs
the just-built bundle on the native-arch runner and runs `flatpak run
com.goshapps.GameHandler --version` (`.github/workflows/release.yml:103-108`)
— the only step in the chain that executes a shipped artifact.

## What is deliberately excluded

- **Build intermediates and prior artifacts.** `target/`, `build-flatpak/`,
  `.flatpak-builder/`, `flatpak-repo/` and `dist/` are all in the manifest
  `skip` list, and the tarball is a whitelist stage — a member exists only
  because an `install -D` put it there — so none of these can leak into an
  artifact. An artifact never embeds a previous artifact.
- **Version control.** `.git` is skipped.
- **The Python reference tree.** `gamehandler/` (the PySide6/QML application
  kept as the parity reference, DECISIONS D-01) is *not* in the `skip` list —
  it is copied into the flatpak-builder context like the rest of the
  checkout — but no module command installs it, so it reaches no artifact.
  The tarball never sees it. `release-check.sh` still reads
  `gamehandler/__init__.py`'s `__version__` as one of the three version
  sources that must agree with the tag (`release-check.sh:46, 67`), which is
  the only release-side purpose the tree serves.
- **Cargo caches.** `CARGO_HOME=/run/build/gamehandler/cargo` lives inside
  the flatpak-builder sandbox and is not exported; nothing under a host
  `~/.cargo` is shipped. The dependency bytes that *are* needed arrive via
  `cargo-sources.json`, pinned by sha256/commit.
- **Everything else in the checkout** — `docs/`, `tests/`, `scripts/`,
  `build-aux/`, the meson files, `rust-toolchain.toml` — is present in the
  dir source but installed by no command, and staged by no tarball step.

## Running it by hand

```bash
# Prerequisites: a built binary for the tarball, flatpak-builder + the
# flathub remote for the bundles, ostree for the ref checks.
cargo build --release --locked

scripts/package-release.sh  --arch x86_64            # dist/…-x86_64.tar.gz
scripts/package-flatpak.sh  --arch x86_64            # dist/…-x86_64.flatpak
# (aarch64 requires an aarch64 host or binfmt; the CI uses native runners)

scripts/verify-release.sh dist/                      # the whole set, or nothing
scripts/publish-release.sh v<VER> dist/              # verify → sums → draft → upload → publish
```

`release-check.sh v<VER>` is the front gate: the tag must agree with the
workspace `Cargo.toml`, the newest metainfo `<release>`, and
`gamehandler/__init__.py` (`release-check.sh:55-73`). `release-notes.sh`
flattens that metainfo `<release>` block into the GitHub Release body so the
store listing and the release page carry the same text
(`release-notes.sh:22-58`). In CI the whole sequence is
`.github/workflows/release.yml`: validate → a two-arch native build matrix
(`ubuntu-24.04` for x86_64, `ubuntu-24.04-arm` for aarch64) → publish, where
`publish-release.sh` keeps the release a draft until every asset is uploaded
and read back non-empty (`publish-release.sh:65-85`).
