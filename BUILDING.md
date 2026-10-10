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

## Interactive audit and isolated launches

`integration_test/audit_test.dart` exercises rendered desktop controls against the
real Rust store. On headless Linux use a session bus and Xvfb:

```sh
export GAMEHANDLER_NATIVE_LIBRARY="$PWD/rust/target/debug/libgamehandler_desktop.so"
xvfb-run -a dbus-run-session -- flutter test integration_test/audit_test.dart -d linux
GAMEHANDLER_NETWORK_AUDIT=1 xvfb-run -a dbus-run-session -- flutter test integration_test/network_audit_test.dart -d linux
```

The network audit opens project URLs and starts/cancels real downloads; it requires
network access and an associated browser. `GAMEHANDLER_AUDIT_RUNNERS_ONLY=1`
limits it to runner controls. Integration builds replace Flutter's debug bundle;
run the desktop build driver again before testing a distributed bundle's CLI and
resources. Do not run desktop integration and a Flutter release build concurrently: they share Linux ephemeral engine files. Finish the build, copy its complete bundle to a separate QA directory, then run integration tests while inspecting that preserved bundle.

The bundled CLI accepts explicit absolute library roots:

```sh
gamehandler-cli --config-dir /path/to/config --data-dir /path/to/data --list
gamehandler-cli --config-dir /path/to/config --data-dir /path/to/data --launch GAME_ID
```

Generated shortcuts retain these roots. Native Linux shortcuts use
`$XDG_DATA_HOME/applications` when XDG_DATA_HOME is absolute. Flatpak uses its
explicitly granted host application directory. Search remains visible after page
navigation; Clear filters resets search and category. A removed default runner
is shown as not installed until a replacement is chosen. Launch minimization keeps
the application reachable from the taskbar and restores it after a launch failure.

Vendor signature verification needs the bundled osslsigncode 2.14 or a compatible
host installation. In this audit, host 2.9 could not decode the signed CRL while
2.14 verified the same official installer. Do not disable signature, revocation,
or TLS checks to work around that failure. Wine32 vendor execution remains an
independent runtime gate. See [the hands-on audit](docs/HANDS_ON_AUDIT.md) for
executed checks, screenshots, defects and exact external blockers.

For hands-on Linux QA in a headless environment, use Xvfb with a window manager
(e.g. XFWM4) and a session D-Bus. Native picker/drag tests also need GTK settings
and a file manager such as Thunar. The optional X11 screenshot/input helpers used
for this audit depend on xdotool, Python Xlib and Pillow; they are QA tools, not
application dependencies. The network test can use
`GAMEHANDLER_AUDIT_BROWSER_DISPLAY=:105` to close each opened Chromium tab on a
separate QA desktop. Start and retain a base browser tab on that display first;
do not drive another window on the same display during that test.
