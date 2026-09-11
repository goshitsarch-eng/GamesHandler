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
import shlex
import subprocess
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


def op_shell_split(args):
    """`shlex.split`: the launch-arguments field.

    A raise is the behaviour under test here, so this returns normally and lets
    `run_case` record the ValueError — the two messages (`No closing quotation`
    and `No escaped character`) are user-visible and the port keeps them
    distinct.
    """
    return shlex.split(_need(args, "text"))


def op_pure_posix_name(args):
    """`PurePosixPath(text).name` — the basename rule, not `rsplit('/')`.

    Reached in the port through `safe_archive_name` and the `umu-run`
    comparison in `uses_proton_runtime`.
    """
    return pathlib.PurePosixPath(_need(args, "text")).name


def op_install_id_from_parts(args):
    """`ReleaseInfo.install_id` built from raw parts, with no asset selection."""
    return runners.ReleaseInfo(
        tag=_need(args, "tag"),
        name="vector",
        download_url="https://example.invalid/x",
        size=0,
        family_id=_need(args, "family_id"),
    ).install_id


def op_wine_prefix_root(args):
    return runners.wine_prefix_root(_need(args, "prefix"))


def op_prefix_drive_cs(args):
    return [str(path) for path in runners.prefix_drive_cs(_need(args, "prefix"))]


def op_readable_error(args):
    """`_readable_error`: a runner's output reduced to the part worth showing.

    Pure, so this op is the shipped code unchanged. It is here because the
    shaping rules are easy to misread — noise prefixes are dropped *unless* they
    are all there is, the tail is taken over four lines, and the result is
    space-joined and then truncated to 240 characters.
    """
    return runners._readable_error(_need(args, "text"))


def op_launch_failure_text(args):
    """The `LaunchedGame.failure()` contract, with B-07's ordering applied.

    **This is the one op that does not answer with the shipped code.** It is the
    shipped code plus the one line the shipped code is missing, and the missing
    line is the entire point of the op.

    `LaunchedGame.failure()` (`runners.py:1358`) reads `self.errors.text()` after
    `process.wait()` returns. `wait()` returning proves the child was *reaped*;
    it says nothing about the stderr pipe having been drained. `_ErrorTail._drain`
    runs on a separate daemon thread with no join, so it can still be
    unscheduled when `failure()` reads the buffer — the buffer looks empty, and
    the real Wine/Proton error text is silently replaced by the generic
    `the runner exited with status {code}`. Measured at 90/300 runs under load
    (FINDINGS B-07), which is why `tests/test_runners.py:761` is flaky.

    So this op builds a *real* `_ErrorTail` over the child's stderr and then does
    the thing the shipped code cannot: it joins the drain thread before reading.
    Everything else — the chunked reads, the bounded buffer, the UTF-8 decoding,
    `_readable_error`, the fallback string — is the real implementation, so the
    answer here is the behaviour the app *means* to have. The port must match
    this, not the buggy ordering; the divergence is in reliability, not in output
    format.

    Not covered: the `TimeoutExpired` arm, which returns `None` for a child that
    is still running. Reaching it needs a clock, and a corpus that depends on
    timing cannot be re-run for an identical answer. It is pinned by a unit test
    on the Rust side instead, and named here so the gap is visible.
    """
    capture = args.get("capture", True)
    process = subprocess.Popen(
        [
            sys.executable,
            "-c",
            "import sys\n"
            f"sys.stderr.write({args.get('stderr', '')!r})\n"
            "sys.stderr.flush()\n"
            f"sys.exit({_need(args, 'exit_code')})\n",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    tail = runners._ErrorTail(process.stderr) if capture else None
    process.wait()
    if tail is not None:
        # The line the shipped `failure()` omits. Without it the read below
        # races the drain thread and loses the text ~30% of the time.
        tail._thread.join()
    code = process.returncode
    if code == 0:
        return None
    detail = runners._readable_error(tail.text()) if tail is not None else ""
    return detail or f"the runner exited with status {code}"


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
    "shell_split": op_shell_split,
    "pure_posix_name": op_pure_posix_name,
    "install_id_from_parts": op_install_id_from_parts,
    "readable_error": op_readable_error,
    "launch_failure_text": op_launch_failure_text,
}


# ---------------------------------------------------------------------------
# The suite: an adversarial corpus, kept as code
# ---------------------------------------------------------------------------
#
# `--suite` writes the case list to stdout, so the committed fixtures are
# *derived* rather than hand-maintained:
#
#     python3 docs/migration/oracle/run_runners_vectors.py --suite \
#         > docs/migration/oracle/fixtures/runners_vectors.cases.json
#     python3 docs/migration/oracle/run_runners_vectors.py \
#         < docs/migration/oracle/fixtures/runners_vectors.cases.json \
#         > docs/migration/oracle/fixtures/runners_vectors.answers.json
#
# The corpus lives here, in reviewable code, rather than in a JSON blob whose
# provenance nobody can check. It is deliberately **adversarial rather than
# representative**: F-H's lesson was that fixtures drawn from tidy inputs pass
# while real data breaks, so the cases below are the ones designed to catch the
# *last* review's class of bug — inputs where the obvious implementation and
# Python's differ — not cases that cover lines.
#
# Six groups earn their size:
#
# * `shell_split` — the hand-written `shlex` port. Every case is a place where a
#   plausible scanner disagrees with CPython: `\x0b` is not whitespace, `''` is
#   a word, single quotes do not escape, `"a\nb"` keeps its backslash, `a\` is
#   an error but `"a\"` is a different one.
# * `asset_name` / `safe_archive_name` / `pure_posix_name` — the `or ""`
#   truthiness rule and the `PurePosixPath` basename rule. Both already found
#   real defects in the port; these are the cases that found them.
# * `asset_matches` / `pick_asset` / `looks_like_archive` /
#   `install_id_from_parts` — asset selection and install ids, where a wrong
#   answer installs the wrong build or silently overwrites another family's.
#   The inline `family` cases exist because no shipped family has both a
#   `require` and an `exclude` that bite.
# * `parse_env_block` / `merge_dll_overrides` / `normalize_desktop_size` /
#   `virtual_desktop_argv` — the pure launch-option inputs, which the toggle
#   matrix will cross next. They are pure, so they are pinned now rather than
#   with `launch_opts`.
# * `readable_error` — how a runner's output is reduced to a toast. The
#   `splitlines` cases are the reason it is here: `str.splitlines()` breaks on
#   `\x0b \x0c \x1c \x1d \x1e \x85 \u2028 \u2029`, and a port that
#   splits on `\n` joins two errors into one line. Same class of trap as
#   shlex's whitespace set.
# * `launch_failure_text` — B-07. The one op here that is not pure; see its own
#   note, which explains why the ordering lives in this script rather than in
#   the shipped `failure()`.
#
# Ten of the sixteen ops are replayed by `crates/core/src/oracle_tests.rs` §12;
# the four launch-option ops are named there as deferred to `launch_opts`, and
# that list is asserted against the corpus so it cannot rot.


def _shell_split_cases():
    """Inputs where a plausible `shlex` reimplementation diverges."""
    texts = [
        # Ordinary shapes, for a baseline.
        "", "   ", "-fullscreen -dx11", " \t-x\r\n-y ", "a b", "trailing  ",
        # Quoting and grouping.
        "a 'b c' d", '"a b"', "'a b'", "''", '""', "a '' b", "-x ''",
        "''''", '""""', "a''b", "'a' 'b'", "''  ''", "' '", '" "', '"\t"',
        'a"b"c', "'a'\"b\"", '"a b"c d', "-b''", "-x=1 2",
        # Whitespace that is *not* in `shlex.whitespace`.
        "\x0b", "a\x0cb", "a\tb", "\x0c", "a\x0b b",
        # Comments are disabled, so `#` is an ordinary character.
        "a#b", "# comment", "a # b", "#a=1",
        # Escapes outside quotes.
        "back\\slash", "a\\ b", "a\\$b", "a\\`b", "-DNAME=\\'v\\'", "\\\\",
        "x\\ y\\ z", "\\'a'", "a\\\\b", 'a\\"b', "\\$HOME",
        # Single quotes are literal — the case a uniform-backslash scanner
        # gets wrong.
        "'a\\b'", "'\\'", "'a\\$'", "'\\$'", "'\\`'",
        # Double quotes escape only the quote and the backslash.
        '"a\\\\b"', '"a\\"b"', '"a\\$b"', '"a\\nb"', '"a\\ b"', '"\\ "',
        "x\"\\ \"", '"\\\\"', '"\\""',
        # Errors: unterminated quotes.
        "'unterminated", '"unterminated', "a'", "a\"b", '"a b', "'", '"',
        '"a\\"', '"a\\\\', "'a\\", "x'y",
        # Errors: a trailing backslash.
        "\\", "a\\", "x y\\", '"a\\', "'a b\\",
        # Both at once, where CPython's precedence is observable.
        "'unterminated\\", '"unterminated\\',
    ]
    return [{"op": "shell_split", "args": {"text": text}} for text in texts]


def _name_cases():
    """The `or ""` truthiness rule and the basename rule."""
    cases = []
    # `asset_name` reads `str(asset.get("name") or "")`, so falsy values of
    # every type collapse to "" and truthy non-strings go through `str()`.
    # `1e-5` is deliberately **absent**. `str(1e-05)` is `'1e-05'` in Python and
    # the port's float writer renders the same value `0.00001` — the notation
    # band recorded with the float-divergence predicate, whose boundary is
    # agreed at `1e-4`. Carrying the case here would make this corpus's text
    # comparison red for a reason it cannot fix, and dropping it silently would
    # hide the difference; it is pinned instead by
    # `oracle_tests::the_notation_band_is_a_known_difference` and by the
    # `exp_notation_boundary` fixtures. `0.0001` is the agreed boundary and
    # agrees exactly, so the band's edge is still covered.
    for value in [
        None, "", "GE-Proton9-5.tar.gz", 0, 1, 0.0, 1.5, False, True, [],
        {}, ["a"], {"a": 1}, -0.0, "  ", "\n", 0.0001,
    ]:
        cases.append({"op": "asset_name", "args": {"asset": {"name": value}}})
    # A missing key is the same branch as an explicit null.
    cases.append({"op": "asset_name", "args": {"asset": {}}})
    # `PurePosixPath(name).name` drops empty and `.` components *before* taking
    # the last one, and keeps a `..`.
    for text in [
        "", "/", "a", "a/b", "trailing/", "a/./b", "a/..", "..", ".",
        "/abs/path", "//double//slash//", "a//b", "./", "../", "a/b/",
        "a/./", "a/b/..", "with space.tar.gz", "\\windows\\path",
        "GE-Proton9-5.tar.gz", "...", "a/...", "a/.b", ".hidden",
    ]:
        cases.append({"op": "pure_posix_name", "args": {"text": text}})
        cases.append(
            {"op": "safe_archive_name", "args": {"name": text, "fallback": "runner.tar.gz"}}
        )
    # The fallback is only used when the name reduces to nothing.
    for name, fallback in [
        ("", "custom.tar.xz"), ("/", "custom.tar.xz"), (".", "custom.tar.xz"),
        ("..", "custom.tar.xz"), ("x", ""), ("", ""),
    ]:
        cases.append(
            {"op": "safe_archive_name", "args": {"name": name, "fallback": fallback}}
        )
    return cases


def _install_id_cases():
    """Install ids, where a collision is a silently broken install."""
    cases = []
    for tag in [
        "GE-Proton9-5", "v1", "", "release/v1", "..", ".", "  spaced  ",
        "a" * 200, "a" * 166, "a" * 167, "tag with spaces", "tag;rm -rf /",
        "-leading-dash", "trailing.", "dots...", "~tilde", "a~b",
        "ünïcøde", "1.2.3", "x" * 180, "x" * 181,
    ]:
        for family_id in ["proton-ge", "wine-vanilla", "proton-cachyos", "system", "x"]:
            cases.append(
                {"op": "install_id_from_parts", "args": {"tag": tag, "family_id": family_id}}
            )
    for install_id in ["", ".", "..", "a/b", "a\\b", ".hidden", "ok", "with space", "/abs"]:
        cases.append({"op": "safe_install_id", "args": {"install_id": install_id}})
    for tag in ["", "..", ".", "/", "\\", "ok", "a b", "a" * 300, "ünïcøde"]:
        cases.append({"op": "sanitise_release_tag", "args": {"tag": tag}})
    return cases


def _selection_cases():
    """Asset selection, crossed so no shipped family's token set is assumed."""
    cases = []
    names = [
        "GE-Proton9-5.tar.gz", "wine-11.15-amd64.tar.xz",
        "wine-11.15-amd64-wow64.tar.xz", "wine-11.15-staging-amd64.tar.xz",
        "wine-11.15-staging-tkg-amd64.tar.xz", "wine-11.15-proton-amd64.tar.xz",
        "proton-cachyos-11.0-v3-x86_64.tar.xz",
        "proton-cachyos-11.0-slr-x86_64.tar.xz",
        "notes.txt", "sha256sums.txt", "a.tar.gz", "a.tgz", "a.tar.xz",
        "a.tar.bz2", "a.zip", "A.TAR.GZ", "x.tar.gz.sig", "",
    ]
    for family_id in [
        "proton-ge", "proton-ge-rtsp", "proton-cachyos", "proton-em",
        "wine-vanilla", "wine-staging", "wine-staging-tkg", "wine-proton",
    ]:
        for name in names:
            cases.append(
                {"op": "asset_matches", "args": {"asset_name": name, "family_id": family_id}}
            )
    # `pick_asset` with an inline family, to reach the branches where `require`
    # and `exclude` both bite — no shipped family has both.
    inline = [
        {"id": "both", "require": ["amd64"], "exclude": ["wow64"]},
        {"id": "prefer", "prefer": ["slr"]},
        {"id": "prefer-missing", "prefer": ["nope"]},
        {"id": "empty", "require": [], "exclude": [], "prefer": []},
        {"id": "case", "require": ["AMD64"], "exclude": ["WOW64"]},
    ]
    for spec in inline:
        for assets in [
            [],
            [{"name": "a.txt"}],
            [{"name": "wine-11.15-amd64.tar.xz"}],
            [{"name": "wine-11.15-amd64.tar.xz"}, {"name": "wine-11.15-amd64-wow64.tar.xz"}],
            [{"name": "wine-11.15-wow64.tar.xz"}, {"name": "wine-11.15-amd64.tar.xz"}],
            [{"name": "a-slr.tar.xz"}, {"name": "b.tar.xz"}],
            [{"name": "A-SLR.tar.xz"}],
            [{"name": None}, {"name": "wine-11.15-amd64.tar.xz"}],
            [{"name": 0}, {"name": "wine-11.15-amd64.tar.xz"}],
            [{"name": "WINE-11.15-AMD64.TAR.XZ"}],
        ]:
            cases.append(
                {"op": "pick_asset", "args": {"assets": assets, "family": spec}}
            )
    for name in names:
        cases.append({"op": "looks_like_archive", "args": {"name": name}})
    return cases


def _launch_option_cases():
    """The pure launch-option inputs the toggle matrix will build on."""
    cases = []
    env_texts = [
        "FOO=1; BAR=two words\n# comment\nBAZ=3",
        "A=1;B=2", "A=1 B=2", "KEY=", "KEY", "=v", " A = b ",
        "export A=1", "A='quoted value';B=2", "A=\"double\";B=2",
        "A='semi;colon'", "A=\"semi;colon\"", "A=a\\b", "A=C:\\path\\to",
        "A='unclosed", "A=\"unclosed", "A=1\n\nB=2", "A=1\r\nB=2",
        "A=1\rB=2", "A=1;;B=2", "  ", "", "A=1;A=2", "A=1 A=2",
        "A=1; # comment\nB=2", "#A=1", "A=1;B", "A='a\"b'", "A=\"a'b\"",
        "A=1;B=2;C=3", "A=\"\";B=''", "A=a b c", "A=1;B='x y';C=3",
    ]
    for text in env_texts:
        cases.append({"op": "parse_env_block", "args": {"text": text}})

    for env, extra in [
        ({}, "d3d12=b"),
        ({}, ""),
        ({}, "   "),
        ({}, ";"),
        ({}, ";;a=b;;"),
        ({"WINEDLLOVERRIDES": "winemenubuilder.exe=d"}, "d3d12=b"),
        ({"WINEDLLOVERRIDES": "x=y;"}, "z=w"),
        ({"WINEDLLOVERRIDES": ""}, "a=b"),
        ({"WINEDLLOVERRIDES": "   "}, "a=b"),
        ({"WINEDLLOVERRIDES": "a=b"}, "a=b"),
    ]:
        cases.append(
            {"op": "merge_dll_overrides", "args": {"env": env, "extra": extra}}
        )

    for value in [
        "1920x1080", "2560 x 1440", "nope", "", "   ", "3840X2160",
        "12x34", "123x456", "123456x123456", "1x1", "111x111",
        "1920x1080x2", "x1080", "1920x", "1920 x1080", " 1920x1080 ",
    ]:
        cases.append({"op": "normalize_desktop_size", "args": {"value": value}})

    for name, size, argv in [
        ("Half-Life", "1280x720", ["/usr/bin/wine", "/g/hl.exe"]),
        ("Half-Life", "", ["/usr/bin/wine", "/g/hl.exe"]),
        ("Half-Life", "bogus", ["/usr/bin/wine"]),
        ("!!!", "", ["wine", "/g/app.exe"]),
        ("a" * 40, "", ["wine", "/g/app.exe"]),
        ("", "", ["wine"]),
        ("", "", []),
        ("ünïcøde game", "1x1", ["wine"]),
        ("name with spaces", "2x2", ["wine", "/g/app.exe"]),
    ]:
        cases.append(
            {
                "op": "virtual_desktop_argv",
                "args": {"name": name, "virtual_desktop_size": size, "argv": argv},
            }
        )
    return cases


def _readable_error_cases():
    """`_readable_error`'s shaping rules, where a plausible reading differs."""
    cases = []
    for text in [
        "", "   ", "\n", "\r\n",
        "wine: cannot find the executable",
        "warn: fixme: only noise\nwine: real failure",
        "warn: only noise", "fixme: a\ntrace: b\ninfo: c\nwarn: d",
        "one\ntwo\nthree\nfour\nfive\nsix",
        "a\r\nb\rc", "  leading and trailing  ",
        "x" * 300, "a" * 100 + "\n" + "b" * 200,
        "NOISE: case", "Fixme: uppercase prefix is not stripped",
        "err:x11drv: real", "   warn: indented noise is still noise",
        "\x00embedded nul", "ünïcøde: failed",
        # `str.splitlines()` is **not** `str.split("\n")`. It also breaks on
        # `\x0b \x0c \x1c \x1d \x1e \x85 \u2028 \u2029`, and a port that
        # reaches for `split('\n')` renders `a\x0bb` as one line where Python
        # sees two and joins them with a space. Same class of trap as shlex's
        # whitespace set, and the same reason it is in this corpus.
        "a\x0bb", "a\x0cb", "a\x1cb", "a\x1db", "a\x1eb",
        "a\x85b", "a\u2028b", "a\u2029b",
        # Non-splitting whitespace, for contrast: a tab is stripped at the
        # edges but never splits, and an embedded NUL is ordinary text.
        "a\tb", "\ta\t", "a\x00b",
        # The noise arm when every line is noise *and* when none is.
        "warn: only\ntrace: only", "err: one\nwarn: two",
        "x" * 240 + "\n" + "y" * 240,
    ]:
        cases.append({"op": "readable_error", "args": {"text": text}})
    return cases


def _launch_failure_cases():
    """B-07: the ordering, not the message.

    Every case here has a **non-empty stderr and a non-zero exit**, which is
    exactly the situation the race corrupts. A case with empty stderr would be
    answered `the runner exited with status N` by both orderings, so it could
    not tell the race from the fix — and a corpus that cannot tell them apart
    is the shape of test that let B-07 live in the Python suite for so long.
    """
    stderrs = [
        "wine: cannot find the executable\n",
        "err:module:import_dll Library not found\n",
        "warn: noise only\nwine: the real one\n",
        "one\ntwo\nthree\nfour\nfive\n",
        "fixme: only noise\n",
        "x" * 300,
        "a" * 100 + "\n" + "b" * 200,
    ]
    cases = [
        {"op": "launch_failure_text",
         "args": {"stderr": text, "exit_code": code, "capture": True}}
        for text in stderrs
        for code in (1, 2, 127)
    ]
    # Exit 0 is `None` regardless of stderr — the "it did not fail" arm.
    cases.append({"op": "launch_failure_text",
                  "args": {"stderr": "noise on a clean exit\n", "exit_code": 0,
                           "capture": True}})
    # No capture at all: `errors is None`, so the fallback is the *only*
    # possible answer. Distinct from an empty buffer, which is the race's
    # signature — and telling those two apart is the point of B-07.
    cases.append({"op": "launch_failure_text",
                  "args": {"stderr": "never read\n", "exit_code": 3,
                           "capture": False}})
    return cases


def build_suite():
    """The whole adversarial corpus, as a list of request objects."""
    cases = []
    cases += _shell_split_cases()
    cases += _name_cases()
    cases += _install_id_cases()
    cases += _selection_cases()
    cases += _launch_option_cases()
    cases += _readable_error_cases()
    cases += _launch_failure_cases()
    return cases


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
    if "--suite" in argv[1:]:
        json.dump({"cases": build_suite()}, sys.stdout, indent=1,
                  ensure_ascii=False)
        sys.stdout.write("\n")
        return 0

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
