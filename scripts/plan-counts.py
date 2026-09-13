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
REPORT = REPO / "docs" / "audit" / "REPORT.md"

# `REPORT.md`'s `Category` column, which names an owner role rather than a
# document. Kept here so the report's table can be generated from the same rows
# the two `PLAN.md` tables come from — the two disagreed (the report said 42
# fixed where the rows said 47, and 5 open `P1` where there were none), and the
# reason is that the report's numbers were maintained by hand from the plan's
# instead of derived from the rows both summarise.
ROLES = {
    "BUG": "Bugs, reliability, feature completeness",
    "ARCH": "Architecture, code quality",
    "UX": "libcosmic / COSMIC UX",
    "PERF": "Performance, resource",
    "SEC": "Security, robustness",
    "PKG": "Packaging, platform, QA",
}
# `REPORT.md`'s order, which is not the alphabetical one the `PLAN.md` family
# table uses.
REPORT_ORDER = ["BUG", "ARCH", "UX", "PERF", "SEC", "PKG"]

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


# The words a status cell may begin with.
#
# This exists because of a failure `--check` **could not see**. A row written as
# ``| **FIXED** ... |`` — the emphasis marks PLAN.md puts around the id in its
# other columns — was parsed, was classified by `settled()` as *not* settled
# (correctly: `"**FIXED**".startswith("FIXED")` is false), and changed no count,
# because a `FIXED` and an `OPEN` row differ in exactly one column of the
# output. So the two tables stayed byte-identical to the file's stale ones,
# every generated line was present verbatim, and `--check` exited 0 while the
# file's own summary was wrong and the row's status unreadable.
#
# That is the defect this whole audit is about — a check that passes without
# inspecting what it claims — sitting in the checker, so an unrecognised status
# is a named problem rather than a silent non-match.
#
# The rule is *begins with*, not *is*, because the file's convention is a status
# word followed by whatever the row needs to say about it: ``FIXED `26d56d3` ``,
# `PARTIAL with BUG-21`, `FIXED — the paragraph was rewritten …`. Those tails
# are prose the tables do not count and must not be read as part of the status.
# What this catches is the word being *dressed up* — emphasised, quoted,
# backticked as a whole, or misspelled — because every one of those makes the
# row's status illegible to `settled()` without making it legible to a reader.
# `CLOSED` is in the list because `BUGS.md` uses it for a row that describes
# correct behaviour rather than a defect (see that row's own note).
STATUS_WORDS = ("FIXED", "PARTIAL", "WITHDRAWN", "OPEN", "CLOSED")


def status_cell(line: str) -> str:
    """The status column of a finding row, backtick marks removed.

    The marks are stripped because they wrap the *commit hash* rather than the
    status — `FIXED `26d56d3`` — so a caller that stripped only the ends would
    see `FIXED `26d56d3`. `parse()` did exactly that, which is harmless for a
    prefix test and would not be for an equality one.
    """
    return line.split("|")[-2].strip().replace("`", "").strip()


def status_problems(plan: str) -> list[str]:
    """Rows whose status cell does not begin with a known status word."""
    problems = []
    section = None
    for number, line in enumerate(plan.splitlines(), start=1):
        header = SECTION.match(line)
        if header:
            section = header.group(1)
            continue
        if line.startswith("## "):
            section = None
            continue
        if section is None or not ROW.match(line):
            continue
        status = status_cell(line)
        word = status.split(" ")[0].rstrip("—-").strip()
        if word not in STATUS_WORDS:
            problems.append(
                f"PLAN.md:{number}: status cell {status[:60]!r} does not begin "
                f"with one of {STATUS_WORDS}. A status wrapped in markup reads "
                f"as un-settled to this script and would leave the summary "
                f"tables stale without `--check` noticing")
    return problems


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
            status = status_cell(line)
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


def tail_lines() -> list[str]:
    """The `Where the tails actually live` table's rows, from the documents.

    That table is a *second* hand-maintained summary in `PLAN.md`, one section
    away from the two this script already computes, and it had gone stale in the
    same way: `ARCHITECTURE.md` was recorded as 7 tails against 8,
    `COSMIC-UX.md` as 1 against 5, `SECURITY.md` as 2 against 3 and
    `PACKAGING.md` as 4 against 5 — every one of them a fix that landed without
    the table being recounted. Its own prose says it was "re-counted here from
    the files on disk", which was true of the revision that wrote it and false
    of every revision after.

    The `Kinds` cell is derived too, and only for the three words the documents
    actually use. The revision this replaces wrote the kind list by hand as
    well, which is how it could say `2 FIXED` about a document with three.

    The row pattern accepts the emphasis and strike-through markers in **either
    order and either count**, because a withdrawn row is written
    ``| ~~**BUG-11**~~ **WITHDRAWN** | … |`` and the first version of this
    pattern only accepted ``| **BUG-nn**``. That version reported `BUGS.md` as
    46 rows where the document holds 47, and the prose below the table was
    written to agree with the wrong number — "`BUGS.md`'s 20 tail-less rows are
    its open `P3` rows", where the twentieth is `BUG-11`'s and the row is
    withdrawn rather than open. A count that cannot see a whole row is the same
    defect as a table maintained by hand, one level down: the number and the
    sentence explaining it were both wrong, and both agreed with each other.
    """
    pattern = re.compile(
        r"^\|\s*(?:~~)?\*{0,2}(?:~~)?`?(BUG|ARCH|UX|PERF|SEC|PKG)-\d+")
    kind = re.compile(r"Status:\s*([A-Z][A-Z ]{2,20})")
    # The table's own order, which is not the alphabetical `FAMILIES` order the
    # two summary tables use. Kept as written so the generated lines can be
    # compared verbatim, which is what makes `--check` able to see a reordered
    # column rather than only a wrong number.
    order = ["BUG", "ARCH", "UX", "SEC", "PKG", "PERF"]
    lines = []
    for family in order:
        document = REPO / "docs" / "audit" / DOCUMENTS[family]
        rows = [line for line in document.read_text(encoding="utf-8").splitlines()
                if pattern.match(line)]
        tailed = [line for line in rows if "Status:" in line]
        counts: dict[str, int] = {}
        for line in tailed:
            match = kind.search(line)
            counts[match.group(1).strip() if match else "?"] = \
                counts.get(match.group(1).strip() if match else "?", 0) + 1
        kinds = ", ".join(f"{count} `{name.strip()}`" for name, count in counts.items())
        lines.append(f"| `{DOCUMENTS[family]}` | {len(rows)} | {len(tailed)} | {kinds} |")
    return lines


def report_tables(rows):
    """`REPORT.md`'s two tables, from the same rows.

    The `Not a defect` column is derived rather than hardcoded at 2: it counts
    the rows `parse()` bucketed as `None`, so a third refuted row reaches the
    report without anyone remembering to change a number.
    """
    refuted = sum(1 for severity, _, _ in rows if severity is None)
    refuted_by_family: dict[str, int] = {}
    for severity, family, _ in rows:
        if severity is None:
            refuted_by_family[family] = refuted_by_family.get(family, 0) + 1

    category = ["| Category | Role | Found | Fixed | Not a defect | Remaining |",
                "|---|---|---|---|---|---|"]
    for family in REPORT_ORDER:
        found, fixed = len([1 for s, f, _ in rows
                            if s is not None and f == family]), \
            len([1 for s, f, st in rows
                 if s is not None and f == family and settled(st)])
        category.append(
            f"| `{family}-xx` | {ROLES[family]} | {found} | {fixed} | "
            f"{refuted_by_family.get(family, 0)} | {found - fixed} |")
    total_found = sum(1 for s, _, _ in rows if s is not None)
    total_fixed = sum(1 for s, _, st in rows if s is not None and settled(st))
    category.append(f"| **Total** | | **{total_found}** | **{total_fixed}** | "
                    f"**{refuted}** | **{total_found - total_fixed}** |")

    severity = ["| Severity | Found | Fixed | Not a defect | Remaining |",
                "|---|---|---|---|---|"]
    for level in SEVERITIES:
        found = sum(1 for s, _, _ in rows if s == level)
        fixed = sum(1 for s, _, st in rows if s == level and settled(st))
        severity.append(f"| {level} | {found} | {fixed} | 0 | {found - fixed} |")
    severity.append(f"| **Total** | **{total_found}** | **{total_fixed}** | **0** | "
                    f"**{total_found - total_fixed}** |")
    return category, severity


def document_statuses() -> dict[str, str]:
    """`{id: STATUS}` from the specialist documents' `Status:` tails."""
    pattern = re.compile(
        r"^\|\s*(?:~~)?\*{0,2}(?:~~)?`?(BUG|ARCH|UX|PERF|SEC|PKG)-(\d+)")
    kind = re.compile(r"Status:\s*([A-Z][A-Z ]{2,20})")
    found: dict[str, str] = {}
    for family, document in DOCUMENTS.items():
        path = REPO / "docs" / "audit" / document
        for line in path.read_text(encoding="utf-8").splitlines():
            match = pattern.match(line)
            if not match:
                continue
            status = kind.search(line)
            found[f"{match.group(1)}-{match.group(2)}"] = (
                status.group(1).strip() if status else "")
    return found


def cross_referenced_pairs(plan: str) -> set[str]:
    """The ids named in `PLAN.md`'s *Cross-referenced pairs* table.

    That table is the audit's record of ids that are **one fix and one
    commit**, and the pairing it describes is this script's own stated
    mechanism; nothing checked it until this function existed. The gap it
    closes was measured, not theorised: the table named eight ids, and two of
    them had drifted — `SEC-05` read `PARTIAL` here and `FIXED` in
    `SECURITY.md`, `UX-26` likewise, each because a later fix moved one cell
    and not the other. Every other id in the table agreed, which is exactly
    why the pair has to be checked as a set rather than sampled.
    """
    ids: set[str] = set()
    inside = False
    for line in plan.splitlines():
        if line.startswith("## "):
            inside = line.strip() == "## Cross-referenced pairs"
            continue
        if not inside:
            continue
        for cell in line.split("|"):
            cell = cell.strip()
            if re.fullmatch(r"`(?:BUG|ARCH|UX|PERF|SEC|PKG)-\d+`", cell):
                ids.add(cell.strip("`"))
    return ids


def pairing_problems(plan: str) -> list[str]:
    """Cross-referenced ids whose two cells disagree about the status word.

    Only the *word* is compared, not the free text after it: the two tails
    describe the fix from their own side and are meant to differ in prose. A
    prefix test is used rather than equality so that `FIXED 75fc739` and
    `FIXED` agree, which is what `status_cell`'s doc already establishes for
    the summary tables.

    `DECISIONS.md` D-59 says a status lives once, as a `Status:` tail on the
    specialist row, with the plan and the report derived from it. The plan's
    cell is derived by hand, so this is the check that the derivation happened.
    """
    document = document_statuses()
    plan_status: dict[str, str] = {}
    inside_findings = False
    for line in plan.splitlines():
        if line.startswith("## "):
            inside_findings = line.strip() == "## Findings"
            continue
        # Scoped to `## Findings` on purpose: the *Cross-referenced pairs*
        # table is itself made of `| \`ARCH-01\` | \`BUG-01\` | …` rows, and
        # the first version of this function read every one of them as a
        # finding row. It reported six disagreements, all of them artefacts of
        # its own parsing — `PLAN.md says 'Architecture'` is a cell of the
        # pairs table, not a status. A checker that miscounts its own input is
        # the same defect as a check that passes without inspecting, one level
        # up.
        if not inside_findings:
            continue
        match = ROW.match(line)
        if match:
            plan_status[f"{match.group(1)}-{match.group(2)}"] = status_cell(line)
    problems = []
    # Every id whose specialist document carries a `Status:` tail is compared,
    # not only the cross-referenced ones. The narrower scope was the first
    # version and it missed the two drifts that motivated the check: `SEC-05`
    # and `UX-26` are not pairs, so comparing pairs alone reported a clean tree
    # while `SEC-05` read `PARTIAL` here and `FIXED` in `SECURITY.md`.
    for identifier in sorted(set(document) & set(plan_status)):
        here = plan_status.get(identifier, "")
        there = document.get(identifier)
        if not here or not there:
            continue
        word_here = here.split(" ")[0].rstrip("—-").strip()
        word_there = there.split(" ")[0].rstrip("—-").strip()
        if word_here != word_there:
            problems.append(
                f"{identifier} is cross-referenced as one fix and one commit, "
                f"but its two status cells disagree: PLAN.md says {word_here!r} "
                f"and its specialist document says {word_there!r}. D-59 puts the "
                f"status once, in the document's `Status:` tail; whichever cell "
                f"is stale, the pair is now recorded twice and differently")
    return problems



def regenerate(path: Path, replacements: list[tuple[str, str]]) -> int:
    """Rewrite the generated lines of `path` in place. Returns lines changed.

    Every `(old, new)` pair is a line-for-line substitution, and the function
    refuses unless the result has **exactly the same number of lines** and every
    line outside the replaced set is byte-identical. That is not defensive
    padding: the first version of this file's editor rewrote `BUGS.md` from its
    cross-reference table to the end and silently dropped 117 lines of findings,
    because the write was `open(p,'w').write(rest)` where `rest` had been built
    from a slice rather than the whole document. `--check` would have caught the
    result on the next run; nothing would have caught it before the commit.

    This exists so that regenerating four summary tables after a status flip is
    one command rather than four hand edits — and the hand edits are what went
    wrong three times this audit, most recently by inventing a count.
    """
    original = path.read_text(encoding="utf-8")
    lines = original.split("\n")
    changed = 0
    for old, new in replacements:
        hits = [index for index, line in enumerate(lines) if line == old]
        if len(hits) != 1:
            raise SystemExit(
                f"{path.name}: expected exactly one line equal to {old[:70]!r}, "
                f"found {len(hits)}. A generated line that is missing was edited "
                f"by hand or never written; fixing it is not this tool's job")
        lines[hits[0]] = new
        changed += 1
    updated = "\n".join(lines)
    if len(updated.split("\n")) != len(lines):
        raise SystemExit(f"{path.name}: the rewrite changed the line count")
    if len(updated.splitlines(True)) != len(original.splitlines(True)):
        raise SystemExit(f"{path.name}: the rewrite changed the line count")
    path.write_text(updated, encoding="utf-8")
    return changed


def generated_replacements(plan: str):
    """`[(old, new)]` for every generated line of `PLAN.md` that is out of date.

    Three of `PLAN.md`'s summaries are generated: the severity table, the family
    table and the tails table. Each is located by its own header or document
    name rather than by line number, so prose moving above it cannot shift the
    substitution onto the wrong line.
    """
    rows, _ = parse(plan)
    severity_table, family_table, _, _ = tables(rows)
    tails = tail_lines()
    lines = plan.split("\n")
    pairs: list[tuple[str, str]] = []

    # `tables()` returns each table as its list of lines, which is what `main`
    # joins to print — not as one string.
    for fresh in (severity_table, family_table):
        if fresh[0] not in plan:
            raise SystemExit(
                f"table header not found verbatim in PLAN.md: {fresh[0]!r}. The "
                f"table was deleted or its header reworded; regenerate by hand")
        start = lines.index(fresh[0])
        current = lines[start:start + len(fresh)]
        if len(current) != len(fresh):
            raise SystemExit(f"table at {fresh[0]!r} is truncated")
        for old, new in zip(current, fresh):
            if old != new:
                pairs.append((old, new))

    for fresh in tails:
        document = fresh.split("`")[1]
        matches = [line for line in lines
                   if line.startswith(f"| `{document}` |") and line.count("|") == 5]
        if len(matches) != 1:
            raise SystemExit(
                f"expected one tails row for {document} in PLAN.md, found "
                f"{len(matches)}")
        if matches[0] != fresh:
            pairs.append((matches[0], fresh))
    return pairs


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true",
                        help="exit non-zero if the file disagrees with its rows")
    parser.add_argument("--write", action="store_true",
                        help="rewrite the generated tables from the rows "
                             "instead of only reporting them stale")
    args = parser.parse_args()

    if args.write:
        plan = PLAN.read_text(encoding="utf-8")
        for line in generated_replacements(plan):
            print(f"PLAN.md: {line[0][:60]!r} -> {line[1][:60]!r}")
        count = regenerate(PLAN, generated_replacements(plan))
        print(f"PLAN.md: {count} line(s) rewritten")
        plan = PLAN.read_text(encoding="utf-8")
        rows, headers = parse(plan)
        report_tables_for_write = report_tables(rows)
        if REPORT.exists():
            report = REPORT.read_text(encoding="utf-8")
            pairs = []
            for fresh in report_tables_for_write:
                start = report.split("\n").index(fresh[0])
                current = report.split("\n")[start:start + len(fresh)]
                pairs += [(old, new) for old, new in zip(current, fresh)
                          if old != new]
            if pairs:
                for line in pairs:
                    print(f"REPORT.md: {line[0][:60]!r} -> {line[1][:60]!r}")
                print(f"REPORT.md: {regenerate(REPORT, pairs)} line(s) rewritten")
        return 0

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

    problems = status_problems(plan)
    problems += pairing_problems(plan)
    tails = tail_lines()
    for line in tails:
        if line not in plan:
            problems.append(
                f"tails-table line not present verbatim in PLAN.md: {line}")
    if REPORT.exists():
        report = REPORT.read_text(encoding="utf-8")
        category, severity = report_tables(rows)
        for line in category + severity:
            if line.startswith("|") and line not in report:
                problems.append(
                    f"REPORT.md table line not present verbatim: {line}")
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
