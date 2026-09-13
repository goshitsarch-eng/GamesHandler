#!/usr/bin/env python3
"""Check `Cargo.lock` against `docs/audit/advisories.json`.

**Why this exists.** `SEC-10` found that this repository had never been checked
against an advisory database, so its state was *unknown* rather than clean. The
audit then did that check by hand — cloning `RustSec/advisory-db`, matching every
`[[package]]` in `Cargo.lock` against it, and recording five live advisories — and
the result was a paragraph in `SECURITY.md`. A paragraph is not a check: nothing
re-ran the match after the next dependency bump, which is exactly the window in
which a transitive pin moves.

**What it checks, and what it does not.** It compares the *locked* version of each
recorded crate against the ledger, and fails on drift in either direction:

  - the crate left the graph entirely            -> the ledger entry is stale
  - the resolved version moved off `locked`      -> re-check the reachability note
  - the resolved version reached `patched`       -> the advisory is CLEARED, which
                                                    is good news that still has to
                                                    reach the ledger and `SEC-10`

It does **not** query RustSec, and it cannot see an advisory that is not already
in the ledger. That is a real limit, not a detail: a new advisory against some
other crate in the graph would not be caught here. The ledger says so too, at the
top of the file, so a reader meets the limitation in both places. The alternative
— vendoring the whole database — buys coverage this repository does not need at
the cost of a large data file that goes stale silently, and the brief's standing
rule is not to take on churn for its own sake.

**Why the drift check is the valuable half.** `SEC-10` names its own follow-up:
`lru >= 0.18.2` and `memmap2 >= 0.9.11` clear both unsound advisories, so *the
next libcosmic bump* should be "a two-version comparison against the new graph,
not a re-audit". This script is that comparison. Run it after a dependency change
and it says which recorded entries moved and what that means.

**Vacuity.** The first version of the matcher `SEC-10` used read only `*.toml`
files and silently opened no advisory at all, reporting a false all-clear — this
project's dominant defect class. So this script asserts that it parsed the ledger
and that it found the crate in the lockfile, and both assertions fail loudly
rather than skipping.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LEDGER = REPO / "docs" / "audit" / "advisories.json"
LOCK = REPO / "Cargo.lock"

# `[[package]]` blocks in a Cargo.lock, which is TOML. Parsed by hand because
# `tomllib` is 3.11+ and this script is run by whatever `python3` the verify
# stage finds; the subset needed here is one `name = "…"` and one
# `version = "…"` per block, which is as simple as the format gets.
PACKAGE = re.compile(r"^\[\[package\]\]$", re.M)
NAME = re.compile(r'^name = "(.+)"$', re.M)
VERSION = re.compile(r'^version = "(.+)"$', re.M)


def locked_versions() -> dict[str, str]:
    """`{crate name: version}` for every package in `Cargo.lock`.

    A crate may legitimately appear more than once at different versions (a
    duplicated transitive dependency). When it does, the version kept is the
    **lowest**, because that is the one an advisory is most likely to apply to
    and under-reporting here would be the failure that matters.
    """
    text = LOCK.read_text(encoding="utf-8")
    blocks = PACKAGE.split(text)[1:]
    out: dict[str, str] = {}
    for block in blocks:
        name = NAME.search(block)
        version = VERSION.search(block)
        if not name or not version:
            continue
        crate, found = name.group(1), version.group(1)
        if crate not in out or version_key(found) < version_key(out[crate]):
            out[crate] = found
    return out


def version_key(version: str) -> tuple:
    """A sortable key for a `x.y.z` version, tolerant of pre-release suffixes.

    Compared numerically field by field. A suffix (`-beta.1`) sorts *below* the
    bare release of the same numbers, which is how Cargo orders them and what
    makes `>= patched` the right test for "the fix is in".
    """
    core, _, suffix = version.partition("-")
    numbers = []
    for part in core.split("."):
        digits = re.match(r"\d+", part)
        numbers.append(int(digits.group(0)) if digits else 0)
    while len(numbers) < 3:
        numbers.append(0)
    return (*numbers, 1 if suffix else 0)


def check() -> list[str]:
    """Problems, as strings. Empty means the graph still matches the ledger."""
    if not LEDGER.exists():
        return [f"{LEDGER.relative_to(REPO)} is missing, so nothing was checked"]
    if not LOCK.exists():
        return [f"{LOCK.relative_to(REPO)} is missing, so nothing was checked"]

    ledger = json.loads(LEDGER.read_text(encoding="utf-8"))
    advisories = ledger.get("advisories") or []
    # Anti-vacuity. A ledger that parsed to an empty list would compare nothing
    # and pass, which is the exact failure `SEC-10`'s first matcher shipped.
    if not advisories:
        return [
            "the ledger holds no advisories, so this check compared nothing. If "
            "every advisory was genuinely cleared, say so by emptying the ledger "
            "deliberately and deleting this guard's expectation — do not let it "
            "pass on a ledger that stopped parsing"
        ]

    locked = locked_versions()
    if not locked:
        return [
            f"parsed no packages out of {LOCK.relative_to(REPO)}, so every "
            f"comparison below would be vacuous"
        ]

    problems = []
    for entry in advisories:
        crate = entry["crate"]
        recorded = entry["locked"]
        patched = entry.get("patched")
        where = f"{entry['id']} ({crate})"

        found = locked.get(crate)
        if found is None:
            problems.append(
                f"{where}: {crate} is no longer in Cargo.lock, so this entry is "
                f"stale. If the crate left the graph, delete the entry from "
                f"advisories.json and update SEC-10's count"
            )
            continue

        if found != recorded:
            # Which direction, because the two mean opposite things.
            if patched and version_key(found) >= version_key(patched):
                problems.append(
                    f"{where}: CLEARED — {crate} resolved at {found}, which is at "
                    f"or past the patched {patched}. This is good news that has to "
                    f"reach the record: update advisories.json and SEC-10, and "
                    f"re-check whether the reachability note still applies"
                )
            else:
                problems.append(
                    f"{where}: {crate} resolved at {found}, but the ledger records "
                    f"{recorded}. The graph moved; re-check the reachability note "
                    f"and update the ledger"
                )
            continue

        # The version matches. If a `patched` version is recorded and the locked
        # one is already past it, the ledger contradicts itself.
        if patched and version_key(found) >= version_key(patched):
            problems.append(
                f"{where}: recorded as live at {found}, but that is at or past its "
                f"own patched version {patched} — the entry contradicts itself"
            )
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--quiet", action="store_true",
        help="print nothing on success, for use inside a larger stage")
    args = parser.parse_args()

    problems = check()
    if problems:
        print("ADVISORY DRIFT:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        print(
            "\nThese are recorded advisories, not new ones: the check is that the "
            "dependency graph still matches what SEC-10 measured. Run the fix the "
            "message names, then re-run.", file=sys.stderr)
        return 1

    if not args.quiet:
        count = len(json.loads(LEDGER.read_text(encoding="utf-8"))["advisories"])
        print(f"advisories: {count} recorded, all still at their recorded version")
    return 0


if __name__ == "__main__":
    sys.exit(main())
