# Building the Flutter desktop prerelease

Use Flutter stable 3.47.7 (Dart 3.13.5), Rust 1.98.1 with clippy/rustfmt,
Python 3.11+, and the checked-in lockfiles. Flutter's desktop prerequisites also
apply: Visual Studio 2022 Desktop development with C++ on Windows; Xcode and
CocoaPods on macOS; Clang, CMake, Ninja, pkg-config, GTK 3, xkbcommon, bz2, lzma
and zlib development packages on Linux. Build on the target OS/architecture.

On Debian/Ubuntu:

```sh
sudo apt-get install clang cmake ninja-build pkg-config libgtk-3-dev libxkbcommon-dev libbz2-dev liblzma-dev zlib1g-dev
flutter pub get
python3 scripts/desktop.py build
python3 scripts/desktop.py run
```

On Windows/macOS run the same commands with the local Python executable. Rust's
GUI-free crate is `rust/Cargo.toml`; root `cargo run` still runs the legacy UI.
The driver builds Rust first and bundles its library and `gamehandler-cli` with
Flutter. Never distribute a Flutter build without running the driver.

```sh
python3 scripts/desktop.py test
python3 scripts/desktop.py package --release
python3 scripts/verify_desktop_bundle.py --release
cargo test --locked -p gamehandler-core
```

`test` runs real ABI tests, Rust tests/Clippy, Flutter analysis, format checks,
widgets and Dart calls into the real shared library. Desktop integration also
requires a graphical session:

```sh
# Linux; use the corresponding .dll/.dylib on Windows/macOS.
export GAMEHANDLER_NATIVE_LIBRARY="$PWD/rust/target/debug/libgamehandler_desktop.so"
flutter test integration_test/desktop_test.dart -d linux
# Headless Linux: xvfb-run -a flutter test integration_test/desktop_test.dart -d linux
```

Linux bundles are tar.gz files, Windows portable bundles and macOS .app bundles
are ZIP files; checksums are adjacent in `dist/`. On Windows, NSIS produces a
per-user installer: `makensis /NOCD packaging/windows/gamehandler.nsi` from the
repository root after a release build. It removes application binaries on
uninstall and retains library/preferences. It accepts an empty installation
folder or an existing installation carrying its own marker. The macOS build is ad hoc signed
for local use; it is not notarized. Production signing needs the maintainer's
certificates, not credentials checked into this repository.

For the new Flatpak, install `flatpak` and `flatpak-builder`, then run
`scripts/build_flatpak.sh`. It uses GNOME 49 (Flutter requires GTK 3), the Wine
25.08 BaseApp, verified osslsigncode and DXVK sources, bundles both stacks,
builds a local repository/bundle, installs it and checks the headless CLI.
It requires working user namespaces and Flathub access. The legacy
`build-aux/flatpak/build.sh` still builds the original application.
The new manifest currently targets Linux x86_64; native ARM bundles are
separate. An installed x86_64 Flatpak passed CLI/cold-start, real portal selection, native
launch and Wine launch checks in this cloud workspace. See docs/desktop-qa.md
for the private D-Bus/tool-path configuration and remaining release gates.

For this managed cloud workspace, source
`/workspace/.gamehandler-tools/activate.sh` before commands. The saved setup
script extracts signed Debian packages into a workspace sysroot because system
directories are read-only. It installs pinned Flutter/Rust and uses workspace
cache/config locations. No HOME override or signature-verification bypass is
required. That sysroot is development tooling, not a distribution artifact.
