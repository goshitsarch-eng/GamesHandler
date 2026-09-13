#!/usr/bin/env python3
"""Recompute `docs/audit/PLAN.md`'s summary tables from its own rows.

**Why this exists.** The two tables in `PLAN.md`'s *Summary* section are the
audit's headline numbers, and they have been wrong three times — each time
because they were maintained by hand beside the rows instead of derived from
them. A summary that drifts from the rows it summarises is the defect this
whole audit exists to find, one level up (`REPORT.md`, *What this audit found
that matters*), so the arithmetic is a script rather than a habit.

**What it reads.** Every table row in `PLAN.md` whose first cell is a bare
finding id, bucketed by the `### Pn` section it sits under and by its id
prefix. Rows under `### Not a defect` are counted in neither table, which is
the point of that section existing.

**What it prints.** The two tables, in the exact shape `PLAN.md` uses, plus
the section-header counts (`### P2 — 53`) so a stale header is visible too.
`--check` exits non-zero when the file disagrees with the rows.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PLAN = REPO / "docs" / "audit" / "PLAN.md"

# A row is `| `ID-nn` | ... |` — the id alone in the first cell, wrapped in
# backticks. Anchored at the start of the line so a row *quoting* an id inside
# prose cannot be mistaken for a row *about* that id.
#
# The `~~` pair is not decoration to be tolerated, it is load-bearing: a
# withdrawn row is struck through, and the first version of this pattern did not
# allow it — so `BUG-11` was silently absent from every count while the file
# visibly held it. A counter that cannot see a whole row is the same defect as a
# table maintained by hand, in the opposite direction: one overstates, this one
# understated, and both are a summary that disagrees with the rows it summarises.
ROW = re.compile(r"^\|\s*(?:~~)?`(BUG|ARCH|UX|PERF|SEC|PKG)-(\d+)`(?:~~)?\s*\|")
SECTION = re.compile(r"^###\s+(P[0-3]|Not a defect)\s*(?:—\s*(\d+))?\s*$")
FAMILIES = ["ARCH", "BUG", "PERF", "PKG", "SEC", "UX"]
DOCUMENTS = {
    "ARCH": "ARCHITECTURE.md",
    "BUG": "BUGS.md",
    "PERF": "PERFORMANCE.md",
    "PKG": "PACKAGING.md",
    "SEC": "SECURITY.md",
    "UX": "COSMIC-UX.md",
}
SEVERITIES = ["P0", "P1", "P2", "P3"]


def settled(status: str) -> bool:
    """Whether a status counts as `Fixed`.

    Only `FIXED` does. `PARTIAL` is deliberately excluded: a half-fixed finding
    is not closed, so it belongs in `Remaining` — this is the rule `PLAN.md`'s
    own prose states, and stating it here rather than inline is what keeps the
    two tables from disagreeing about it.
    """
    return status.startswith("FIXED") or status.startswith("WITHDRAWN")


def parse(plan: str):
    """`(severity, family, status)` per row, plus the section headers seen."""
    section = None
    headers: list[tuple[str, int | None]] = []
    rows: list[tuple[str | None, str, str]] = []
    for line in plan.splitlines():
        header = SECTION.match(line)
        if header:
            section = header.group(1)
            value = int(header.group(2)) if header.group(2) else None
            if section != "Not a defect":
                headers.append((section, value))
            continue
        # A new `##` section ends the row tables; nothing after it is a finding.
        if line.startswith("## "):
            section = None
            continue
        row = ROW.match(line)
        if row and section is not None:
            cells = line.split("|")
            status = cells[-2].strip().strip("`").strip()
            rows.append((None if section == "Not a defect" else section,
                         row.group(1), status))
    return rows, headers


def tables(rows):
    by_severity = {level: [0, 0] for level in SEVERITIES}
    by_family = {family: [0, 0] for family in FAMILIES}
    for severity, family, status in rows:
        if severity is None:
            continue
        for bucket in (by_severity[severity], by_family[family]):
            bucket[0] += 1
            if settled(status):
                bucket[1] += 1

    def render(pairs, label, extra):
        """One summary table, in the exact shape `PLAN.md` writes it.

        `extra` is the per-row cell `PLAN.md` carries and this script does not
        compute — `Document` in the family table. Reproducing the shape rather
        than only the numbers is what makes `--check` able to compare line for
        line: a check that compared numbers parsed back out of the table would
        pass on a table whose columns had been reordered, which is the same
        "passes without inspecting" shape this script exists to avoid.
        """
        head = f"| {label} |"
        rule = "|---|"
        if extra:
            head += f" {extra} |"
            rule += "---|"
        head += " Findings | Fixed | Withdrawn | Remaining |"
        rule += "---|---|---|---|"
        out = [head, rule]
        for name, document, (found, fixed) in pairs:
            cells = f"| {name} |"
            if extra:
                cells += f" {document} |"
            out.append(f"{cells} {found} | {fixed} | 0 | {found - fixed} |")
        found = sum(b[0] for _, _, b in pairs)
        fixed = sum(b[1] for _, _, b in pairs)
        total = "| **Total** |"
        if extra:
            total += " |"
        out.append(f"{total} **{found}** | **{fixed}** | **0** | "
                   f"**{found - fixed}** |")
        return out

    severity_table = render(
        [(level, "", tuple(by_severity[level])) for level in SEVERITIES],
        "Severity", None)
    family_table = render(
        [(f"`{family}-xx`", f"`{DOCUMENTS[family]}`", tuple(by_family[family]))
         for family in FAMILIES],
        "Family", "Document")
    return severity_table, family_table, by_severity, by_family


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true",
                        help="exit non-zero if the file disagrees with its rows")
    args = parser.parse_args()

    plan = PLAN.read_text(encoding="utf-8")
    rows, headers = parse(plan)
    severity_table, family_table, by_severity, by_family = tables(rows)

    defects = sum(1 for severity, _, _ in rows if severity is not None)
    refuted = sum(1 for severity, _, _ in rows if severity is None)

    print("\n".join(severity_table))
    print()
    print("\n".join(family_table))
    print()
    print(f"rows: {len(rows)}  defects: {defects}  not-a-defect: {refuted}")

    problems = []
    for section, stated in headers:
        if stated is None:
            continue
        actual = by_severity[section][0]
        if stated != actual:
            problems.append(
                f"`### {section} — {stated}` but the section holds {actual} rows")
    for line in severity_table + family_table:
        if line.startswith("|") and line not in plan:
            problems.append(f"table line not present verbatim in PLAN.md: {line}")

    if problems:
        print("\nDISAGREEMENTS:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        if args.check:
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
