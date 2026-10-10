# Contributing

For the Flutter rewrite, follow [BUILDING.md](BUILDING.md) and run
`python3 scripts/desktop.py test` plus the desktop integration test on your OS.
Keep domain/persistence policy in Rust and presentation/lifecycle in Dart, as
specified in [ARCHITECTURE.md](ARCHITECTURE.md). Preserve the JSON schema and
corrupt-file protection; test behavior through the real boundary. Add manual
QA results to [docs/desktop-qa.md](docs/desktop-qa.md) without converting an
unrun platform into a passing claim. The workflow matrix must pass before a
`desktop-vVERSION` prerelease is published. Production signing is a maintainer
step. The legacy verification instructions below still apply to shared core
changes and compatibility regressions.

## Setup

You need Rust 1.93 or newer. With rustup, `rust-toolchain.toml` pins the same
compiler the Flatpak uses (1.98.1, with clippy and rustfmt). For a Flatpak
build you also need `flatpak`, `flatpak-builder`, and the Freedesktop 25.08
runtime/SDK with the `rust-stable` extension — `build-aux/flatpak/build.sh`
installs the Flatpak-side dependencies itself.

Python 3 is needed too: the reference implementation in `gamehandler/` and its
suite under `tests/` are part of the verification chain, and the oracle
fixtures are generated with it.

## Build and run

```bash
cargo build
cargo run                     # the GUI
cargo run -- --list           # the CLI works headless
```

## Verify

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
python3 -m unittest discover -s tests -t .
```

Or run the whole pipeline at once — this is the same chain `scripts/ci.sh`
invokes, and it is the bar for merging:

```bash
bash scripts/verify.sh --help          # lists every stage and flag
bash scripts/verify.sh --skip-flatpak  # everything except the Flatpak build
```

## Things that will bite you

- **`crates/core` has no GUI dependency, deliberately.** It must stay
  headless — the test suite runs without a display because of it. UI-facing
  code goes in `crates/app`.
- **The credits table is mirrored.** `crates/core/src/credits.rs` and
  `gamehandler/credits.py` must have the same section and entry counts, and
  the acknowledgements section of `README.md` is `credits.markdown()` output
  verbatim — a test compares it byte for byte. Edit the data, regenerate the
  README block (`python3 -c "from gamehandler.credits import markdown;
  print(markdown(), end='')"`), and update both files together.
- **Oracle fixtures are generated, not edited.** If you change behaviour the
  Python reference defines, regenerate `docs/migration/oracle/fixtures/` with
  `docs/migration/oracle/gen_oracle.py` and `run_runners_vectors.py` — the
  `oracle-freshness` stage fails on stale fixtures.
- **`cargo-sources.json` must match `Cargo.lock`.** After changing
  dependencies, regenerate `build-aux/flatpak/cargo-sources.json`; the
  `cargo-sources-fresh` stage checks it.
- **Keep lint clean.** `clippy -D warnings` and `fmt --check` are pipeline
  stages, not suggestions.

## Scope

Bug fixes, runner catalogue updates, installer fixes, and documentation
corrections are all welcome. Behaviour the Python reference defines should
stay in parity unless the change is a deliberate divergence — those are
recorded in `docs/audit/DECISIONS.md` and `docs/migration/DECISIONS.md`.

Issues and pull requests: <https://github.com/goshitsarch-eng/GamesHandler/issues>
