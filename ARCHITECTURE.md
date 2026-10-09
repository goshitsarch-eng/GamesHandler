# Desktop architecture

Flutter/Dart owns the shell, widgets, navigation, menus, focus, theme, keyboard
commands, dialogs, drag/drop, native file/URL plugins and window size. Rust owns
the game schema, persistence, queries, validation, executable inspection, verified
HTTP downloads, safe archives, process launch policy and background jobs.

`lib/bridge/backend.dart` calls three versioned C ABI symbols exported by
`rust/src/lib.rs`: ABI version, request, and free. A request is one UTF-8 JSON
operation with explicit absolute configuration/data roots. Responses contain
`ok` and either `data` or a typed error (`validation`, `corrupt_data`,
`unsupported`, `operation`). Input is capped at 8 MiB. Rust catches unwinds;
Dart owns its input allocation, Rust owns its output allocation, and Dart frees
every output through the Rust allocator in a `finally` block. Neither language
passes object references or callbacks across the ABI. ABI version 1 is checked
before the first operation.

Blocking requests and JSON decoding run in a Dart isolate. Slow downloads and
installers run in Rust worker threads and return a job id immediately. The UI
polls job snapshots once per second, exposes progress/cancellation and refreshes
after terminal state changes. Search is debounced; stale response tokens and
mounted/disposed checks prevent old replies from replacing new presentation
state. Settings writes are serialized in Dart. Rust serializes mutations within
a process and uses an OS file lock across new desktop/CLI instances. Legacy
clients do not participate in this lock; do not run them against the same files
while editing in the new application.

The bridge is deliberately hand-written: three stable symbols avoid a generated
object graph, generator/runtime version coupling and hundreds of tiny calls.
There is no bridge-generation build step. `scripts/desktop.py` builds both
stacks and copies the matching library and headless CLI automatically. The
packaged-library verifier checks the ABI from the actual bundle.

The existing pure core is reused behind this boundary. Linux-only Wine/Proton,
archive, installer and plugin modules are compiled only on Linux. Shared cover
art, JSON, models and paths compile on Windows and macOS too. The HTTP trait is
shared; the portable runner compatibility module exposes only the legacy HTTP
names, never a fake Wine implementation. Networking retains HTTPS-only,
certificate-verified transport; vendor installers retain Authenticode checks.

No database/schema conversion is needed: the 31-field game JSON and existing
settings schema remain compatible. Corrupt source files are refused before
mutation. Window size lives separately in `window.json`, because it is UI state.
The previous Rust/libcosmic and Python implementations remain available as
behavior references during migration. They are not dependencies of the Flutter
frontend. See MIGRATION_AUDIT.md for remaining verification work.
