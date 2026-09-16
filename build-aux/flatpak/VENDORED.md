# Vendored files

## `flatpak-cargo-generator.py`

| | |
|---|---|
| Upstream | `flatpak/flatpak-builder-tools`, `cargo/flatpak-cargo-generator.py` |
| Pinned commit | `f03a673abe6ce189cea1c2857e2b44af2dd79d1f` (2025-08-16, "cargo: unwrap tomldoc") |
| Fetched | 2026-09-16 from `https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/f03a673abe6ce189cea1c2857e2b44af2dd79d1f/cargo/flatpak-cargo-generator.py` |
| License | MIT (`__license__ = "MIT"` at the top of the file) |
| SHA-256 | `b373c8ab1a05378ec5d8ed0645c7b127bcec7d2f7a1798694fbc627d570d856c` |
| Bytes / lines | 511 lines |

The file is kept **byte-identical to upstream** so a re-vendor is a diff away:

```bash
curl -fsSL "https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/<commit>/cargo/flatpak-cargo-generator.py" \
    -o build-aux/flatpak/flatpak-cargo-generator.py
sha256sum build-aux/flatpak/flatpak-cargo-generator.py   # update the row above
```

Its runtime dependencies are declared in the file's own PEP 723 header
(`aiohttp`, `PyYAML`, `tomlkit`); `scripts/verify.sh`'s `cargo-sources-fresh`
stage finds it via `find_cargo_generator`, which prefers this vendored copy
over host-wide installs. Regeneration is still a network operation — the
generator fetches every git source's tarball to hash it — which is why the
stage may `SKIP` rather than fail offline.
