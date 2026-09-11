"""Answer `core::runners` vector queries from the *Python* implementation.

This is the executable ground truth for the pure functions in
`gamehandler/runners.py` — the ones whose behaviour is a claim that nobody
should port from a reading, mine or the coordinator's. It is generated from the
Python code for the same reason `gen_oracle.py` is: so the answers do not encode
our *assumptions* about Python's behaviour. F-H is the standing lesson — a
fixture drawn from tidy inputs passed while real data broke.

**It is the companion to `gen_oracle.py`, not a second oracle.** `gen_oracle.py`
owns the file-shaped fixtures (`.in.json` / `.out.json` byte pairs). This script
owns the *function-shaped* ones: a question goes in on stdin, the answer comes
out on stdout, and no files are written. That makes it usable two ways:

    # hand-check a single case
    echo '{"op": "sanitise_release_tag", "args": {"tag": "release/v1"}}' \\
        | python3 docs/migration/oracle/run_runners_vectors.py

    # a whole adversarial suite, as a list
    python3 docs/migration/oracle/run_runners_vectors.py < cases.json

Both emit a JSON array of result records, one per request, in request order:

    [{"op": "sanitise_release_tag", "args": {...}, "ok": true,
      "result": "release-v1~h1a2b3c4d5e6f"}]

A case whose Python raises is reported as `{"ok": false, "error": "ValueError:
..."}` rather than failing the run: for `safe_install_id` the raise *is* the
behaviour under test, and a script that exited non-zero on it could not be used
to generate those cases at all. A request that cannot be *dispatched* (unknown
op, missing argument) is a bug in the caller and exits 2.

Declared in DECISIONS D-06 alongside `gen_oracle.py`. Requires nothing but the
stdlib and this repository; the Python app is imported from the repo root.

**Deterministic by construction.** `runners.py` does not call `time.time()`
outside `launch`'s grace-period bookkeeping, and nothing dispatched here touches
a clock, a network or the filesystem — every op below is a pure function of its
arguments. The clock is frozen anyway, because `virtual_desktop_argv` takes a
`Game` and `Game`'s `added` field defaults to `time.time()`: constructing one
without freezing would make the run depend on when it ran, and a suite that
cannot be re-run for an identical answer is not an oracle. `Game.id` is likewise
generated at random, so every constructed game is given an explicit id —
otherwise two runs would differ in bytes nobody looks at, which is exactly the
kind of difference that hides a real one.
"""
import json
import pathlib
import re
import sys
import time as _time

# Freeze the clock *before* the app is imported: `Game.added` defaults to
# `time.time()`. See the module docstring.
FROZEN_NOW = 1_700_000_000.0
_time.time = lambda: FROZEN_NOW

REPO = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO))

from gamehandler import runners
from gamehandler.models import Game

assert _time.time() == FROZEN_NOW, "clock freeze failed; results would be nondeterministic"


# ---------------------------------------------------------------------------
# Argument handling
# ---------------------------------------------------------------------------

class DispatchError(Exception):
    """A malformed request — the caller's bug, not a behaviour under test."""


def _need(args, name):
    if name not in args:
        raise DispatchError(f"missing required argument {name!r}")
    return args[name]


def _family(args):
    """Resolve `family_id` or an inline `family` object to a RunnerFamily.

    Both are supported on purpose. A `family_id` exercises the real catalogue
    the app ships, which is what catches a mistranscribed token. An inline
    `family` lets a case cross tokens that no shipped family combines — and it
    is the only way to reach the branches where `require` and `exclude` both
    bite, since no real family has both.
    """
    if "family_id" in args:
        return runners.family_by_id(args["family_id"])
    if "family" in args:
        spec = args["family"]
        return runners.RunnerFamily(
            id=spec.get("id", "vector"),
            name=spec.get("name", "Vector"),
            description=spec.get("description", ""),
            github=spec.get("github", "example/example"),
            kind=spec.get("kind", "wine"),
            maintainer=spec.get("maintainer", ""),
            require=tuple(spec.get("require", ())),
            exclude=tuple(spec.get("exclude", ())),
            prefer=tuple(spec.get("prefer", ())),
            when_to_use=spec.get("when_to_use", ""),
        )
    raise DispatchError("need either 'family_id' or 'family'")


def _asset(args):
    """An asset as GitHub would return it, from either a name or a raw object."""
    if "asset" in args:
        return args["asset"]
    if "asset_name" in args:
        return {"name": args["asset_name"]}
    raise DispatchError("need either 'asset' or 'asset_name'")


# ---------------------------------------------------------------------------
# The ops
# ---------------------------------------------------------------------------

def op_safe_archive_name(args):
    return runners.safe_archive_name(_need(args, "name"), args.get("fallback", "runner.tar.gz"))


def op_safe_install_id(args):
    return runners.safe_install_id(_need(args, "install_id"))


def op_sanitise_release_tag(args):
    return runners.sanitise_release_tag(_need(args, "tag"))


def op_release_install_id(args):
    info = runners.ReleaseInfo(
        tag=_need(args, "tag"),
        name=args.get("name", "vector"),
        download_url=args.get("download_url", "https://example.invalid/x"),
        size=args.get("size", 0),
        family_id=args.get("family_id", "proton-ge"),
    )
    return info.install_id


def op_release_size_mb(args):
    info = runners.ReleaseInfo(
        tag=args.get("tag", "v1"),
        name="vector",
        download_url="https://example.invalid/x",
        size=_need(args, "size"),
    )
    return info.size_mb


def op_asset_matches(args):
    return runners.asset_matches(_need(args, "asset_name"), _family(args))


def op_looks_like_archive(args):
    return runners._looks_like_archive(_need(args, "name"))


def op_pick_asset(args):
    # Returns the chosen *name* rather than the whole asset: the rest of the
    # dict is passed through untouched, so echoing it would bulk out every
    # vector with data the function never reads.
    assets = [_asset({"asset": asset}) for asset in _need(args, "assets")]
    picked = runners.pick_asset(assets, _family(args))
    return None if picked is None else picked.get("name")


def op_asset_name(args):
    return str(_need(args, "asset").get("name") or "")


def op_family_by_id(args):
    return runners.family_by_id(_need(args, "family_id")).id


def op_family_urls(args):
    family = _family(args)
    return {"releases_url": family.releases_url, "homepage": family.homepage}


def op_runner_guides(args):
    return [list(row) for row in runners.runner_guides()]


def op_runner_guide_details(args):
    return [
        {
            "title": row.title,
            "kind": row.kind,
            "advice": row.advice,
            "maintainer": row.maintainer,
            "homepage": row.homepage,
        }
        for row in runners.runner_guide_details()
    ]


def op_parse_env_block(args):
    return runners.parse_env_block(_need(args, "text"))


def op_merge_dll_overrides(args):
    env = dict(_need(args, "env"))
    runners.merge_dll_overrides(env, _need(args, "extra"))
    return env


def op_normalize_desktop_size(args):
    return runners.normalize_desktop_size(_need(args, "value"))


def op_virtual_desktop_argv(args):
    # A `Game` is required, so one is constructed with an explicit id — see the
    # module docstring on why the id is never left to the generator.
    game = Game(
        id=args.get("id", "0" * 32),
        name=_need(args, "name"),
        virtual_desktop_size=args.get("virtual_desktop_size", ""),
    )
    return runners.virtual_desktop_argv(list(_need(args, "argv")), game)


def op_wine_prefix_root(args):
    return runners.wine_prefix_root(_need(args, "prefix"))


def op_prefix_drive_cs(args):
    return [str(path) for path in runners.prefix_drive_cs(_need(args, "prefix"))]


OPS = {
    "safe_archive_name": op_safe_archive_name,
    "safe_install_id": op_safe_install_id,
    "sanitise_release_tag": op_sanitise_release_tag,
    "release_install_id": op_release_install_id,
    "release_size_mb": op_release_size_mb,
    "asset_matches": op_asset_matches,
    "looks_like_archive": op_looks_like_archive,
    "pick_asset": op_pick_asset,
    "asset_name": op_asset_name,
    "family_by_id": op_family_by_id,
    "family_urls": op_family_urls,
    "runner_guides": op_runner_guides,
    "runner_guide_details": op_runner_guide_details,
    "parse_env_block": op_parse_env_block,
    "merge_dll_overrides": op_merge_dll_overrides,
    "normalize_desktop_size": op_normalize_desktop_size,
    "virtual_desktop_argv": op_virtual_desktop_argv,
    "wine_prefix_root": op_wine_prefix_root,
    "prefix_drive_cs": op_prefix_drive_cs,
}


# ---------------------------------------------------------------------------
# Driver
# ---------------------------------------------------------------------------

def run_case(case):
    if not isinstance(case, dict):
        raise DispatchError(f"a case must be an object, got {type(case).__name__}")
    op = case.get("op")
    if op not in OPS:
        raise DispatchError(f"unknown op {op!r}; known ops: {', '.join(sorted(OPS))}")
    args = case.get("args", {})
    if not isinstance(args, dict):
        raise DispatchError(f"'args' must be an object, got {type(args).__name__}")
    record = {"op": op, "args": args}
    try:
        record["ok"] = True
        record["result"] = OPS[op](args)
    except DispatchError:
        raise
    except Exception as exc:
        # A raise is often the behaviour under test, so it is a result.
        record["ok"] = False
        record["error"] = f"{type(exc).__name__}: {exc}"
    return record


def main(argv):
    raw = sys.stdin.read()
    try:
        document = json.loads(raw) if raw.strip() else {"cases": []}
    except json.JSONDecodeError as exc:
        print(f"stdin is not valid JSON: {exc}", file=sys.stderr)
        return 2

    if isinstance(document, dict) and "cases" in document:
        cases = document["cases"]
    elif isinstance(document, dict):
        cases = [document]
    elif isinstance(document, list):
        cases = document
    else:
        print("stdin must be a case object, a list of cases, or {\"cases\": [...]}",
              file=sys.stderr)
        return 2

    try:
        results = [run_case(case) for case in cases]
    except DispatchError as exc:
        print(f"bad request: {exc}", file=sys.stderr)
        return 2

    json.dump(results, sys.stdout, indent=1, sort_keys=False, ensure_ascii=False)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
