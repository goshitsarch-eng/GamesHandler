"""Test-only Kirigami stub, loaded only when the QML smoke test runs.

Unittest discovery imports packages even when their names do not match the
test-file pattern. Keep this entry point free of PySide6 imports so headless
hosts can discover the suite and skip the optional QML smoke test normally.
"""


def register() -> None:
    """Load the Python types and register the QML-file halves of the stub."""
    from ._types import register as register_types

    register_types()
