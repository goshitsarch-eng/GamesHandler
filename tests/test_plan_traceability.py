"""Every task worked on has a row in the plan, and every id the plan names exists.

Finding #71: **T-26 and T-28 were real tasks and neither had a row in
`docs/migration/PLAN.md`.** T-26 was created, worked and *landed* (`41a6612`,
the Plugins page) with no row anywhere; `git log -S` says it never had one. So
the plan could not be read to find out what was being built, and nothing
noticed, because nothing read the plan. T-04's own row is the proof that this
is load-bearing rather than tidiness: it is the one row the project's critical
path runs through, and it was reachable only because someone had written it.

This is the #67 shape as well — three malformed rows in the same table survived
every check, for the same reason.

Two directions, because a table rots in whichever one nobody looks at:

1. **commit -> plan.** Every `T-nn` named in a commit subject has a row. This is
   what would have caught #71: the commit `T-26: the Plugins page…` existed
   while the row did not.
2. **plan -> plan.** Every `T-nn` *mentioned* anywhere in PLAN.md has a row. A
   mention with no row is a task the plan hands work to and never defines —
   which is not hypothetical: it is how `T-63` reads today, named by T-04's row
   as the owner of the `.desktop` writer (`P-71`) and defined nowhere.

# Why `T-63` is deferred rather than fixed

`docs/migration/PLAN.md` is not this file's to edit (D-05: one agent per file,
and it belongs to the Lead), so the dangling mention is recorded in
[`UNROUTED`] with its finding number and reported rather than silently
tolerated. The entry is checked in the same pass that uses it, so a list item
that stops being true fails the suite — the same rule `PINNED_PENDING` states in
its own header, and the reason deferrals do not rot into a place where problems
are forgotten.

# Two parsing traps, both measured rather than assumed

- **A `|` inside a code span still delimits a GFM table cell.** PLAN.md's rows
  are dense with backticked code, and one of them contains a literal pipe
  written as `\\|`. Splitting on `|` alone invents cells and shifts every
  column; the split has to be `re.split(r'(?<!\\\\)\\|', line)`. #67 recorded
  this trap, and the lead reproduced the defect in a first draft of #71.
- **Task ids have a letter suffix.** `T-01a` is a row (the compatibility
  oracle), and it is a *different task* from `T-01`. A bare `T-\\d{2}` pattern
  matches `T-01` inside `T-01a`, so a missing `T-01a` row would resolve against
  T-01's row and pass. The id pattern carries the suffix, and the mutation test
  below pins that it does.

# What this checks, and what it does not

It checks that ids *resolve*. It does not check that a row's `DONE (sha)` names
a commit that exists, that a row's status matches what happened, or that a row's
scope is the work that was done — those are claims about content, and #66/#68
are the record of how wrong content claims get. It is a reachability check, and
it is named for that.

**And it only reads `T-nn`.** `P-nn` (parity) and `F-nn`/`R-nn` ids are not
covered by either direction, which is a bound worth stating because it is
exactly how `T-63` stayed invisible to the commit half: the work landed as
`P-71: the .desktop writer…` (`b9e8cc6`) while T-04's row is the only place
`T-63` is named. Widening the pattern to every id family would mean checking
that `P-` ids resolve too, which is a different table (#67's) and a different
task; recorded here as a limit rather than left to read as coverage.

# The third direction: source -> plan (T-39)

Every `TODO(T-nn)` in `crates/` must name an id that has a row in `PLAN.md`,
plus a report of which ids carry tags. It would have caught `T-38`'s absence:
eight `TODO(T-13)` tags pointed at an id nothing defined until `84d367e`.

**The bound, and it is the point of the task: passing means "no tag is
dangling", not "every task is wired".** A tag is a promise that the work
belongs to a task, and this check can only prove the id *names* a row. Whether
that row's declared scope actually contains the work is a judgement over two
passages of prose — the row's scope string and the code the tag sits in — and
it is deliberately **not** mechanised. A check that claimed to make it would be
#69's shape: a check narrower than the claim printed beside it. So the rot
Architecture found in #79 — installer work tagged `T-13`, cover work tagged
`T-11`, the file chooser tagged `T-09` — passes here exactly as a correct tag
does, because every one of those ids resolves. The failure mode to guard
against is a reader taking a green run for a completeness proof, which is how
#66 survived; the report below exists so a human can do the part that needs a
human, by comparing a handful of ids against their scopes.

# Table integrity (#84)

A literal `|` inside a markdown table cell splits the row, and the split is
**silent** — the row still renders. `9eaf11d` shipped such a row (a quoted Rust
snippet, `DEFAULT_TOGGLES.iter().map(|(key, _, _)| *key)`); it measured 9 parts
where the table declared 7, it was committed, and it was green. The same
mistake was made twice more in one session, once by the lead in `PLAN.md`.

**Two corrections to the obvious implementation, both measured rather than
reasoned.** The specification was "every row line of a table has the same
number of `\\|`-split parts as its header". Counting parts verbatim is wrong in
both directions on today's `PLAN.md`:

  * It is **red on a valid row**. `PLAN.md:325` omits its trailing `|`, which
    GFM permits — a row needs neither edge pipe. It splits to 5 parts against a
    6-part header and reads as malformed while rendering exactly as intended.
  * It is **blind to a real one**. `PLAN.md:352` has **5 cells in a 4-column
    table** and splits to exactly 6 parts, because the extra cell and the
    missing trailing pipe cancel. The parity hides it.

So [`cells`] strips the optional edge pipes before splitting, and the check is
on **cell counts**, which catches the second and clears the first. This is not
a cosmetic difference: rendered, the T-09 row produces four `<td>` cells and
the `**REMAINDER, per D-52 (#68b)…**` paragraph — the note that T-09 is LANDED
but its scope is unmet — is **dropped**, because GFM ignores cells past the
header's column count.

The count to compare against is read from each table's **own header row**, not
hardcoded: a table that legitimately gains a column must not read as broken,
which is this defect class wearing the other hat. Two documents are covered —
`PLAN.md` and `VERIFY-FINDINGS.md` — so this file reads the lead's second doc
as well. It reads both; it edits neither. A malformed row is **reported, never
repaired** from inside the test: the cell is the owner's to rewrite with the
pipe escaped, and a test that edited the document it is checking could report a
tree it had just made clean.

The count to compare against is read from each table's **own header row**, not
hardcoded: a table that legitimately gains a column must not read as broken,
which is this defect class wearing the other hat. Two documents are covered —
`PLAN.md` and `VERIFY-FINDINGS.md` — so this file reads the lead's second doc
as well. It reads both; it edits neither. A malformed row is **reported, never
repaired** from inside the test: the cell is the owner's to rewrite with the
pipe escaped, and a test that edited the document it is checking could report a
tree it had just made clean.

# The other half, and #93's own statement of it

**A row with well-formed cells and no table is not a table, and #84's check
could not tell the difference.** Counting cells against a header presupposes
there is a header. On this tree there was not, for four runs of 56 lines: the
whole second half of `PLAN.md`'s task table (T-12…T-39, cut off from `:341` by
prose inserted at `:355`) and three runs of findings (61–80, 81–82, 83–88),
every row of them with a perfect cell count and none of them in a table. UX
found the findings half, the lead measured the `PLAN.md` half and filed #93,
and `7d8176e` inserted the four missing header+delimiter pairs. Measured three
ways at `7d8176e` — the structural rule in [`tables`] (a table is a run whose
second line is a `|---|` delimiter row), `markdown-it`'s `gfm-like` preset, and
Python-Markdown's `tables` extension — all four runs then rendered as tables.

The durable check is [`headerless_runs`] and
`test_every_row_run_is_immediately_headed`: a run of rows breaks on any
non-matching line **including a blank one**, and its header must be *immediately*
above it. **The lead's first scanner searched backwards past the blank line for
the nearest header and reported all four broken runs as headed** — it is
recorded as the wrong instrument in #93, and the reason is not bookkeeping: in a
document made of tables, "a header exists somewhere above this line" is true of
almost every line, so the predicate stops discriminating.

Two renderers are not quoted as agreeing with each other in general: they differ
by one table in `PLAN.md` — Python-Markdown reports 14 where markdown-it reports
15, merging the two tables at `:225` and `:243` that a heading separates with no
blank line between them, which is that extension's limit rather than a GFM
behaviour. [`is_delimiter`] is that structural rule, and it is what the checks
use; the renderers are how the rule was confirmed rather than what implements
it.

# What the headers did not fix (#94)

The four headers were taken from "each file's own first well-formed table
rather than retyped" (`7d8176e`), and for `PLAN.md` the first well-formed table
is the legend at `:12` — `| Doc | Owner | Contents |`. That is a three-cell
header over `T-12`…`T-40`, whose rows have four cells, so GFM renders three
columns and **drops the Notes cell of all 29 rows**: rendered, T-12 produces
three `<td>` where T-01 produces four, and the Notes text is simply absent. It
is #84's defect class again — silent, because the row still renders — and it is
recorded in [`MISHEADED_TABLES`] rather than fixed here, because `PLAN.md` is
the lead's file (D-05). The rule it illustrates is the one both findings share:
a table has one header, and copying one from elsewhere in the file is retyping
with extra steps.
"""

import re
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PLAN = ROOT / "docs" / "migration" / "PLAN.md"
#: Read for its tables only (#84). This file writes to neither document.
FINDINGS = ROOT / "docs" / "migration" / "VERIFY-FINDINGS.md"

# `T-01`, `T-26`, `T-01a`. The optional suffix is load-bearing — see the module
# docstring — and `\b` on both ends keeps `T-12` from matching inside a longer
# token such as a hypothetical `T-123`.
TASK_ID = re.compile(r"\bT-\d{2}[a-z]?\b")

# GFM: a literal pipe inside a code span is written `\|` and does not delimit a
# cell. Splitting on every `|` invents cells; #67 is the record of that.
CELL = re.compile(r"(?<!\\)\|")

# A row of the task table. `cell[0]` is empty — the line starts with `|` — and
# `cell[1]` is the id column. Prose rows, `P-` rows, `F-` rows and `R-` rows are
# first cells too, and none of them is a `T-` id, so the pattern excludes them
# without a second condition to keep in step.
MIN_ROWS = 30
MIN_COMMITS = 100
MIN_COMMIT_IDS = 10
MIN_MENTIONS = 30

# T-39. The tag itself, anchored on `TODO(`: a bare `T-10` in prose is a
# reference, not a tag, and 75 such tokens sit in `crates/` today. Group 1 is
# the id, so the message can say what was looked up.
TAG = re.compile(r"TODO\((T-\d{2}[a-z]?)\)")

# The same pattern as a grep ERE, for `git grep -E`. Kept beside `TAG` so the
# two cannot drift.
TAG_SOURCE = r"TODO\(T-[0-9]{2}[a-z]?\)"

# Floors for the tag direction. These are a **tripwire on a scan that has
# stopped matching**, not a budget: a scan rooted somewhere unexpected returns
# zero tags and would pass every assertion below on nothing at all.
#
# They sit far below the real count on purpose, because the count is falling
# fast and legitimately — 32 tags when this was written, 19 when the direction
# first landed, 13 while a neighbouring agent consolidated `main.rs` — so a
# floor near the measurement turns a landed task into a red suite. What the low
# number cannot distinguish, a real defect cannot exploit either: an id with no
# row is reported whatever the population, and a scan that returns nothing is
# the only way to make the check vacuous.
MIN_TAGS = 4
MIN_TAG_IDS = 2
MIN_TAG_FILES = 2

# Floors for the table direction, set below the pair's total for the same
# reason. Measured at `f051b82`, counting only runs that are actually tables:
# PLAN.md 15 tables / 144 body rows, VERIFY-FINDINGS.md 2 / 34 — **17 tables and
# 178 rows** together. The other 4 runs, 56 lines, are not tables at all and are
# named in [`MALFORMED_RUNS`]; under the earlier rule that called every run a
# table these parsed as 230 rows (171 + 59), and the comment here used to say
# 264. That 264 is not reproducible and was carried for several commits without
# being re-measured, which is why the numbers above name the sha they came from.
MIN_TABLES = 12
MIN_TABLE_ROWS = 150

# Floors for #93's run check, per document. A run of id-bearing rows is the unit
# the rule is stated over, and a parser that found no runs would report every
# document headed. Measured at `bc47b41`: PLAN.md 2 runs / 40 rows,
# VERIFY-FINDINGS.md 4 runs / 76 rows.
MIN_PLAN_RUNS = 2
MIN_PLAN_RUN_ROWS = 20
MIN_FINDINGS_RUNS = 3
MIN_FINDINGS_RUN_ROWS = 40

#: Rows that are malformed on this tree today, keyed by `(document, first cell)`
#: and carrying the finding that owns each.
#:
#: **A deferral, not an exemption**, and checked rather than promised: this check
#: cannot land green while a real malformed row is in a document it may not edit
#: (`PLAN.md` is the lead's, D-05), and a red gate blocks three other agents on a
#: file that is not mine to fix. So the row is *reported* — named in the test's
#: output with its finding — and the assertion stays live for every other row.
#:
#: Keyed on the first cell rather than a line number, because a line number in a
#: document this size is stale within the hour and this file already records that
#: (`KNOWN_DEAD`'s deleted entry cited `main.rs:1339`, which had been `1043`).
#: An entry that stops describing a malformed row — because the row was fixed, or
#: the table was rewritten — fails `test_the_malformed_row_list_is_not_stale`, so
#: the list cannot outlive the defect.
MALFORMED_ROWS = {
    (PLAN.name, "T-09"): (
        "#84. The T-09 row has **5 cells in a 4-column table** and the extra one is "
        "silently dropped when rendered: GFM ignores cells past the header's count, so "
        "the `**REMAINDER, per D-52 (#68b)**` paragraph — the note that T-09 is LANDED "
        "while its scope is not met — does not appear in the table. Measured by "
        "rendering the table: 4 `<td>` cells, and `none asks whether its P-items are "
        "met` absent from the HTML. Left as a deferral because `docs/migration/PLAN.md` "
        "is the lead's file (D-05); the fix is theirs, and it is to merge the two "
        "paragraphs into the Notes cell (or add the fifth column to the header) rather "
        "than to escape a pipe — this is an extra cell, not a split one. Delete this "
        "entry when the row is fixed."
    ),
}

#: Whole tables whose header disagrees with every row under it, keyed by
#: `(document, first body row's first cell)` with the finding that owns each.
#:
#: **A sibling of [`MALFORMED_ROWS`] rather than 29 entries in it.** One wrong
#: header makes every row of its table malformed at once, and 29 entries that
#: all say the same sentence would bury the finding instead of recording it. The
#: key names the table by its first body row — `T-12` — because that survives
#: the header being rewritten, which is exactly the edit that deletes this entry.
#:
#: **A deferral, not an exemption**, on [`MALFORMED_ROWS`]' terms: the table is
#: named in the test's output with its finding, and
#: `test_the_misheaded_table_list_is_not_stale` fails if an entry stops naming a
#: table whose rows disagree with its own header.
MISHEADED_TABLES = {
    (PLAN.name, "T-12"): (
        "#94. **The fix for #93 put the wrong header on half the task table.** "
        "`PLAN.md:361` was given `| Doc | Owner | Contents |` — three cells, and the "
        "legend table's header from `:12`, not this table's `| # | Task | Owner | "
        "Notes |`. Every row from T-12 to T-40 has four cells, so GFM renders three "
        "columns and **drops the Notes cell of all 29 rows**: measured by rendering, "
        "T-12 produces 3 `<td>` where T-01 produces 4, and the Notes text "
        "`OWNERSHIP CORRECTED` is absent from the output. This is #84's own defect "
        "class (silent, because the row still renders) and it is worse than the "
        "paragraph it replaced, where every cell was lost but nothing claimed "
        "otherwise. **Fix: replace `:361` with `| # | Task | Owner | Notes |`** — the "
        "same header the table's first half uses at `:341`. Delete this entry when "
        "the header matches."
    ),
}

#: Pipe-runs that are not tables, keyed by `(document, first cell)` with the
#: finding that owns each.
#:
#: **EMPTY, and that is the point.** The four runs this listed until `7d8176e` —
#: `PLAN.md:361` (T-12…T-39) and `VERIFY-FINDINGS.md:79`, `:432`, `:479` — were
#: given headers by the lead, so every run in both documents is now a table and
#: `test_every_row_run_is_immediately_headed` passes on its own merits rather
#: than through this list. The mechanism stays because the next orphan has to be
#: deferrable without weakening the check, and because an empty list that the
#: staleness test still checks is what makes the difference between "nothing is
#: broken" and "nobody looked" visible in the code.
MALFORMED_RUNS = {}


#: Task ids PLAN.md names but never defines, with the finding that owns each.
#:
#: **A deferral, not an exemption.** An entry that stops being a dangling
#: mention — because the row was written, or the mention was removed — fails
#: [`PlanTraceabilityTests.test_the_deferral_list_is_not_stale`], so this cannot
#: become a list of things nobody looks at again.
#:
#: One entry right now (`T-63`), and the header used to read "EMPTY right now"
#: over it — a claim about this dict written by hand rather than derived from
#: it, in the file that disproves it. Nothing checked the prose, so it stayed
#: wrong from the commit that introduced the entry. That is why the count is
#: not repeated here: [`PlanTraceabilityTests.test_the_deferral_list_is_not_stale`]
#: is what actually holds the list honest, and a reader wanting the count can
#: read the dict below.
#:
#: If you are here to add one, the cheaper fix is almost always to write the
#: missing row.
UNROUTED = {
    "T-63": (
        "#71 (second instance). T-04's row names it as the owner of the `.desktop` "
        "writer — `T-63` is P-71, the desktop-shortcut writer, and T-04's row "
        "sequences the whole critical path around it, and now says the work is "
        "DONE — `T-63`/`.desktop` landed at `b9e8cc6` (606 lines, "
        "`crates/core/src/runners/desktop.rs`). So this is not only a mention with "
        "no row: it is #71's shape a second time, a task worked and landed with no "
        "row anywhere, and the id it landed under was `P-71`, which the commit half "
        "of this check cannot see because it reads `T-nn` only. "
        "No row defines it: `grep -c "
        "'^| T-63 |' docs/migration/PLAN.md` is 0, and the mention at PLAN.md:347 "
        "is the only one outside this file. (This sentence used to read \"`git "
        "grep T-63` over tracked files returns PLAN.md:347 and nothing else\" — "
        "false the moment this file was written, since the sentence itself and six "
        "others in it contain the string. A claim broader than what was measured, "
        "in the file that falsifies it; scoped to outside-this-file, which is what "
        "was actually being claimed.) It is "
        "deferred rather than fixed because `docs/migration/PLAN.md` is the Lead's "
        "file (D-05, one agent per file), and it is deferred rather than ignored "
        "because an id the plan hands work to and never defines is work with no "
        "owner. Delete this entry when the row is written."
    ),
}


def table_ids(plan_text):
    """The task ids that have a row, out of PLAN.md's text."""
    found = {}
    for number, line in enumerate(plan_text.splitlines(), start=1):
        if not line.startswith("|"):
            continue
        cells = CELL.split(line)
        if len(cells) < 2:
            continue
        match = TASK_ID.fullmatch(cells[1].strip())
        if match:
            found.setdefault(match.group(0), number)
    return found


def mentioned_ids(plan_text):
    """Every task id the text names, anywhere — rows, prose, cross-references."""
    return {match.group(0) for match in TASK_ID.finditer(plan_text)}


def commit_subjects():
    """Every commit subject in this checkout's history, oldest last."""
    try:
        result = subprocess.run(
            ["git", "log", "--format=%s"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError:
        raise AssertionError(
            "`git` is not on PATH, so the commit half of this check cannot run. "
            "This test exists to compare the plan against what was actually "
            "committed; without git it would assert over nothing and pass, which "
            "is the defect it was written to catch. Run it from a checkout with "
            "git available."
        )
    except subprocess.CalledProcessError as exc:
        raise AssertionError(
            f"`git log` failed in {ROOT} with exit {exc.returncode}: "
            f"{exc.stderr.strip()}. This test cannot run outside a git "
            "checkout, and must not silently pass there."
        )
    return [line for line in result.stdout.splitlines() if line.strip()]


def commit_ids(subjects):
    """Task ids named in commit subjects, each mapped to one such subject."""
    found = {}
    for subject in subjects:
        for match in TASK_ID.finditer(subject):
            found.setdefault(match.group(0), subject)
    return found


def unrouted(subjects, plan_text, deferred=None):
    """`(id, reason)` for every task id that is named but has no row.

    `reason` is the commit subject or the line of PLAN.md that named it, so the
    failure can say where to look rather than only what is missing.
    """
    deferred = UNROUTED if deferred is None else deferred
    rows = table_ids(plan_text)
    problems = []

    for task, subject in sorted(commit_ids(subjects).items()):
        if task not in rows and task not in deferred:
            problems.append((task, f"commit: {subject}"))

    lines = plan_text.splitlines()
    mentions = {}
    for number, line in enumerate(lines, start=1):
        for match in TASK_ID.finditer(line):
            mentions.setdefault(match.group(0), number)
    for task, number in sorted(mentions.items()):
        if task not in rows and task not in deferred:
            problems.append((task, f"PLAN.md:{number} (no row defines this id)"))

    return problems


def todo_tags(root="crates/"):
    """`(tag, id, path, line)` for every `TODO(T-nn)` under `root`.

    Read with `git grep` so the scan covers tracked files only, which is what
    "in the source" means here. The exit code is the trap: git grep exits **1
    on no matches** — a legitimate result that must not read as a failure — and
    **128 outside a repository**, which must not read as success. So 0 and 1 are
    accepted and anything else raises, the same handling `commit_subjects` gives
    `git log`.
    """
    try:
        result = subprocess.run(
            ["git", "grep", "-nE", TAG_SOURCE, "--", root],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
    except FileNotFoundError:
        raise AssertionError(
            "`git` is not on PATH, so the tag half of this check cannot run. It "
            "exists to compare the plan against the tags in the source; without "
            "git it would assert over nothing and pass, which is the defect it "
            "was written to catch."
        )
    if result.returncode not in (0, 1):
        raise AssertionError(
            f"`git grep` over {root} exited {result.returncode}: "
            f"{result.stderr.strip()}. Exit 1 means no tags, which is a real "
            "answer; anything else means the scan did not run, and a scan that "
            "did not run reports every tag as resolved."
        )

    tags = []
    for entry in result.stdout.splitlines():
        # `path:line:content`; the content holds the colons, so split twice.
        path, line, content = entry.split(":", 2)
        for match in TAG.finditer(content):
            tags.append((match.group(0), match.group(1), path, int(line)))
    return tags


def dangling_tags(tags, rows):
    """`(tag, id, path, line)` for every tag whose id has no row."""
    return [entry for entry in tags if entry[1] not in rows]


def cells(line):
    """The cells of a GFM table row, edge pipes removed.

    A row's leading and trailing `|` are both optional in GFM, so counting them
    is counting layout rather than columns — and counting them *verbatim* is
    wrong in both directions on this tree. Measured on `PLAN.md`: the valid
    `:325` (trailing pipe omitted) splits to 5 parts against a 6-part header and
    reads as malformed, while the genuinely malformed `:352` (5 cells in a
    4-column table) splits to exactly 6 because its extra cell and its missing
    trailing pipe cancel. Removing the edges first makes the count a property of
    the row's columns, which is the thing that splits.
    """
    body = line.strip()
    if body.startswith("|"):
        body = body[1:]
    if body.endswith("|") and not body.endswith("\\|"):
        body = body[:-1]
    return CELL.split(body)


#: A GFM delimiter row: pipes, dashes, colons and spaces, at least one dash. A
#: table is a header line, this row, then body rows, all left-aligned with no
#: blank line between them.
DELIMITER = re.compile(r"[\s|:-]+")


def is_delimiter(line):
    """True for the `|---|---|` row that makes a pipe-run a table."""
    return "-" in line and DELIMITER.fullmatch(line) is not None


def pipe_runs(text):
    """`[(line_number, line)]` per run of consecutive `|`-starting lines.

    A helper for both directions below, so "run" means one thing: the unit that
    a blank line or a paragraph breaks. A run is not necessarily a table — see
    [`headerless_runs`], which is the half of #84 that has no header to count
    against.
    """
    lines = text.splitlines()
    runs, run = [], []
    for number, line in enumerate(lines, start=1):
        if line.startswith("|"):
            run.append((number, line))
        elif run:
            runs.append(run)
            run = []
    if run:
        runs.append(run)
    return runs


def tables(text):
    """`(header_line, n_cells, [(line, n_cells, first_cell)])` per *table*.

    A table is a pipe-run whose second line is a delimiter row. The header row
    is the run's first line, which is where the expected column count comes from
    — a number read from the table rather than written here, so a table that
    gains a column is not reported as broken.

    The delimiter row is dropped: it is dashes and colons, it carries no cells to
    disagree about, and including it would make the count depend on how many
    dashes an author wrote.
    """
    out = []
    for run in pipe_runs(text):
        if len(run) < 2 or not is_delimiter(run[1][1]):
            continue
        header_line, header = run[0]
        n_cells = len(cells(header))
        rows = [
            (number, len(cells(row)), cells(row)[0].strip())
            for number, row in run[2:]
            if not is_delimiter(row[1])
        ]
        out.append((header_line, n_cells, rows))
    return out


def headerless_runs(doc, text):
    """`(doc, first_line, n_lines, first_cell, n_ids)` per run that is not a table.

    **The half of #84 the header comparison cannot see, and the rule #93 states
    as its own finding: a run of well-formed rows with no table above it is not
    a table.** A run whose second line is not a delimiter row is a paragraph to
    every GFM renderer, so every row in it loses its columns and there is no
    header for [`malformed_rows`] to count against — the rows still *look* like
    table rows in the source, which is the same reading-satisfies-the-author
    defect as a dropped cell, one level up.

    Three details, each of them a defect this function was written against:

    * **The run breaks on any non-matching line, including a blank one.** That
      is what [`pipe_runs`] does by construction: a run is a maximal block of
      consecutive `|`-starting lines.
    * **The header must be *immediately* above — there is no searching backwards
      past a blank line.** The lead's first scanner did search backwards and
      reported all four broken runs as headed; it is recorded as the wrong
      instrument in #93, and the wrongness is not academic, because "a header
      exists somewhere above this" is true of nearly every line in a document
      made of tables, so the predicate stops discriminating.
    * **A header that is present but the wrong width is *not* this check's** —
      that is [`malformed_rows`], which counts cells against it. Measured on
      this tree at `bc47b41`: `PLAN.md:361` was headed by nothing (reported
      here, deferred in [`MALFORMED_RUNS`]); at the next commit it was headed by
      a 3-cell row over 4-cell rows (reported there, deferred in
      [`MISHEADED_TABLES`]). Both were rendering defects and they have different
      causes, which is why one function does not try to report both.

    `n_ids` counts the rows in the run whose first cell is a task or finding id
    — `T-nn` in `PLAN.md`, a bare integer in `VERIFY-FINDINGS.md` — because that
    is the measure of what the loss costs, and it is what the failure message
    leads with.
    """
    return [
        (
            doc,
            run[0][0],
            len(run),
            cells(run[0][1])[0].strip(),
            sum(1 for _, line in run if is_row_id(cells(line)[0].strip())),
        )
        for run in pipe_runs(text)
        if len(run) < 2 or not is_delimiter(run[1][1])
    ]


def is_row_id(first_cell):
    """True for a cell that can only be a table's first column.

    `T-01`, `T-01a`, or a bare integer — the two documents' row id families.
    Prose never starts a line with either, which is what makes this a usable
    discriminator for "content at stake" rather than a guess.
    """
    return (
        re.fullmatch(r"T-\d{2}[a-z]?", first_cell) is not None
        or first_cell.isdigit()
    )


def id_runs(text):
    """`(first_line, n_lines)` per maximal run of id-bearing rows.

    #93's own unit: a run of lines that can only be table rows, broken by any
    line that is not one — including a blank line. Each must be immediately
    preceded by a header and a delimiter row, which is what
    [`headerless_runs`] checks; this exists separately because it is the
    population the check has to have *found* for its verdict to mean anything,
    and a scan that matches nothing reports a clean document.
    """
    runs, run = [], []
    for number, line in enumerate(text.splitlines(), start=1):
        if line.startswith("|") and is_row_id(cells(line)[0].strip()):
            run.append(number)
        elif run:
            runs.append((run[0], len(run)))
            run = []
    if run:
        runs.append((run[0], len(run)))
    return runs


def malformed_rows(doc, text):
    """`(doc, line, expected, got, first_cell, table_key)` for every row off its header.

    `table_key` is the first body row's first cell — how a table is named in
    [`MISHEADED_TABLES`]. A line number would not do: the header being fixed is
    the edit that deletes the entry, and it moves nothing, but the rows around
    it move constantly.
    """
    out = []
    for _, n_cells, rows in tables(text):
        key = rows[0][2] if rows else ""
        out += [
            (doc, line, n_cells, got, first, key)
            for line, got, first in rows
            if got != n_cells
        ]
    return out


def live_malformed_rows(doc, text):
    """[`malformed_rows`] minus the rows the two deferral lists name.

    One filter, used by both the gate and the test that drives it, so a mutation
    test cannot pass by filtering differently from the check it is testing.
    """
    return [
        row
        for row in malformed_rows(doc, text)
        if (row[0], row[4]) not in MALFORMED_ROWS
        and (row[0], row[5]) not in MISHEADED_TABLES
    ]



class PlanTraceabilityTests(unittest.TestCase):
    def setUp(self):
        self.plan = PLAN.read_text(encoding="utf-8")
        self.findings = FINDINGS.read_text(encoding="utf-8")
        self.subjects = commit_subjects()

        # Anti-vacuity (#32/#43). A parser that stops matching reports a clean
        # plan, and every assertion below would be trivially satisfied on an
        # empty parse. These bounds are deliberately far below the real counts
        # — 34 rows, 211 commits, 22 ids, 40 mentions when this was written —
        # so they fail on a broken parse rather than on real growth.
        rows = table_ids(self.plan)
        self.assertGreaterEqual(
            len(rows),
            MIN_ROWS,
            f"parsed only {len(rows)} task rows out of {PLAN.name} — the table "
            f"parser has stopped matching, and a check with no rows finds no "
            f"missing rows.",
        )
        self.assertGreaterEqual(
            len(self.subjects),
            MIN_COMMITS,
            f"`git log` returned only {len(self.subjects)} commit subjects. "
            f"Either this is not the full checkout or git is being run "
            f"somewhere unexpected; the commit half of this check would be "
            f"vacuous.",
        )
        self.assertGreaterEqual(
            len(commit_ids(self.subjects)),
            MIN_COMMIT_IDS,
            "almost no commit subject names a task id, so the commit half of "
            "this check is not checking anything. Every task in this project is "
            "committed as `T-nn: …`; if that convention changed, this test needs "
            "to change with it rather than pass.",
        )
        self.assertGreaterEqual(
            len(mentioned_ids(self.plan)),
            MIN_MENTIONS,
            f"found only {len(mentioned_ids(self.plan))} task ids mentioned in "
            f"{PLAN.name}. The reachability half of this check has nothing to "
            f"check.",
        )

        # T-39's floors. A `git grep` rooted somewhere unexpected returns zero
        # tags, and every assertion below would be satisfied by an empty list.
        tags = todo_tags()
        self.assertGreaterEqual(
            len(tags),
            MIN_TAGS,
            f"found only {len(tags)} `TODO(T-nn)` tags under crates/. A scan that "
            f"found no tags reports every tag as resolved, which is the vacuous "
            f"pass this floor exists to prevent.",
        )
        self.assertGreaterEqual(
            len({tag[1] for tag in tags}),
            MIN_TAG_IDS,
            f"the tags under crates/ name only {len({tag[1] for tag in tags})} "
            f"distinct ids. Either they have been consolidated or the scan is "
            f"matching something other than what it thinks.",
        )
        self.assertGreaterEqual(
            len({tag[2] for tag in tags}),
            MIN_TAG_FILES,
            "the tags under crates/ come from fewer than "
            f"{MIN_TAG_FILES} files, so the scan is not walking the tree.",
        )

        # #84's floors, over both documents.
        documents = [(PLAN.name, self.plan), (FINDINGS.name, self.findings)]
        n_tables = sum(len(tables(text)) for _, text in documents)
        n_rows = sum(len(rows) for _, text in documents for _, _, rows in tables(text))
        self.assertGreaterEqual(
            n_tables,
            MIN_TABLES,
            f"parsed only {n_tables} tables out of {PLAN.name} and "
            f"{FINDINGS.name}. The table parser has stopped matching, and a check "
            f"with no tables finds no malformed rows.",
        )
        self.assertGreaterEqual(
            n_rows,
            MIN_TABLE_ROWS,
            f"parsed only {n_rows} table rows across the two documents, so the "
            f"integrity check is reading almost nothing. Rows are what carry the "
            f"cells; without them there is nothing to disagree with a header.",
        )

        # #93's floors. The run check asserts that every run is headed, and its
        # verdict is empty when the parser finds no runs — so the population it
        # looked at is asserted here, in the same pass that produces the verdict.
        # Measured at `bc47b41`: PLAN.md 2 runs of 11 + 29 rows, VERIFY-FINDINGS
        # 4 runs. The floors sit below that for the usual reason.
        for name, text, min_runs, min_ids in (
            (PLAN.name, self.plan, MIN_PLAN_RUNS, MIN_PLAN_RUN_ROWS),
            (FINDINGS.name, self.findings, MIN_FINDINGS_RUNS, MIN_FINDINGS_RUN_ROWS),
        ):
            runs = id_runs(text)
            self.assertGreaterEqual(
                len(runs),
                min_runs,
                f"found {len(runs)} runs of id-bearing rows in {name}; the run "
                f"check would call a document clean on the strength of very "
                f"little. A document made of tables has runs in it.",
            )
            self.assertGreaterEqual(
                sum(n for _, n in runs),
                min_ids,
                f"the {len(runs)} runs in {name} hold only {sum(n for _, n in runs)} "
                f"rows, so the header above each one governs almost nothing.",
            )

    def test_every_task_a_commit_names_has_a_row(self):
        """#71: T-26 was created, worked and landed with no row anywhere."""
        problems = [
            (task, where)
            for task, where in unrouted(self.subjects, self.plan)
            if where.startswith("commit:")
        ]
        self.assertEqual(
            [],
            problems,
            "these task ids are named in a commit subject but have no row in "
            f"docs/migration/{PLAN.name}:\n"
            + "\n".join(f"  {task} — {where}" for task, where in problems)
            + "\n\nA task that is worked on is a task the plan must define: add "
            "the row, or fix the id in the commit's own message if it is a typo. "
            "This is finding #71, which is how T-26 got to be landed and "
            "undefined.",
        )

    def test_every_id_the_plan_names_has_a_row(self):
        """A mention with no row hands work to a task that does not exist."""
        problems = [
            (task, where)
            for task, where in unrouted(self.subjects, self.plan)
            if where.startswith("PLAN.md:")
        ]
        self.assertEqual(
            [],
            problems,
            "these task ids are named in "
            f"docs/migration/{PLAN.name} but no row defines them:\n"
            + "\n".join(f"  {task} — {where}" for task, where in problems)
            + "\n\nEither write the row or point the mention at the id that "
            "actually owns the work. An id the plan uses and never defines is "
            "work with no owner, which is how #71's T-26 stayed invisible.",
        )

    def test_the_deferral_list_is_not_stale(self):
        """`UNROUTED` entries are checked, so the list cannot rot.

        An entry that no longer describes a dangling mention has to be deleted:
        either the row was written or the mention was removed, and both are
        good news that the list is hiding.
        """
        live = {task for task, _ in unrouted(self.subjects, self.plan, deferred={})}
        stale = sorted(task for task in UNROUTED if task not in live)
        self.assertEqual(
            [],
            stale,
            f"these UNROUTED entries no longer describe a dangling mention: "
            f"{stale}. The row was written, or the mention was removed — either "
            f"way, delete the entry. A deferral list that is not checked is how "
            f"a deferral list turns into a place where problems are forgotten.",
        )

    def test_the_check_notices_a_removed_row_and_a_dangling_mention(self):
        """The check above, run against inputs that must fail it (#26).

        A source-reading check that has only ever been seen to pass is a check
        whose failure branch nobody has watched work. This drives the same
        `unrouted()` the tests above use, over a plan with one row deleted and a
        plan with one fabricated mention, and requires each to be reported —
        then requires the *unmutated* plan to report neither in the same run, so
        the comparison is not two runs of different code.
        """
        baseline = unrouted(self.subjects, self.plan)
        self.assertEqual(
            [], baseline, f"this test needs a clean baseline to mutate from; got {baseline}"
        )

        # A real row, removed — the #71 defect exactly: the commit stands and
        # the row does not.
        row = next(
            line
            for line in self.plan.splitlines()
            if line.startswith("| T-26 |")
        )
        without_row = self.plan.replace(row + "\n", "")
        self.assertNotEqual(without_row, self.plan, "the mutation did not change the plan")
        removed = unrouted(self.subjects, without_row)
        self.assertTrue(
            any(task == "T-26" for task, _ in removed),
            f"removing T-26's row was not reported. Everything that was "
            f"reported: {removed}. The commit `T-26: the Plugins page…` is still "
            f"in `git log`, so the check is not reading commit subjects.",
        )

        # A mention of an id that has no row, fabricated — T-63's shape.
        dangling = self.plan + "\nSee T-99 for the detail.\n"
        reported = unrouted(self.subjects, dangling)
        self.assertTrue(
            any(task == "T-99" for task, _ in reported),
            f"a fabricated `T-99` mention was not reported. Everything that was "
            f"reported: {reported}. The plan->plan direction is what catches a "
            f"task the plan hands work to and never defines.",
        )

        # And the suffix trap the module docstring names. `T-01a` is a separate
        # task from `T-01`, so a pattern without the letter suffix matches
        # `T-01` *inside* `T-01a` and resolves the dangling id against T-01's
        # row — reporting the wrong task, or nothing.
        #
        # The row has to be removed *and* the id mentioned somewhere else. Just
        # deleting the row proves nothing, and the first version of this test
        # did exactly that and failed: the row is the only place `T-01a` is
        # named, so deleting it removes the mention too and there is no dangling
        # reference left to report. The scenario is a cross-reference surviving
        # the row — which is also how `T-63` reads today.
        row_01a = next(
            line for line in self.plan.splitlines() if line.startswith("| T-01a |")
        )
        without_01a = (
            self.plan.replace(row_01a + "\n", "") + "\nThe oracle is T-01a's.\n"
        )
        suffix = unrouted(self.subjects, without_01a)
        reported = sorted(task for task, _ in suffix)
        self.assertIn(
            "T-01a",
            reported,
            f"a dangling `T-01a` mention was not reported as `T-01a`; got "
            f"{reported}. `T-01a` and `T-01` are different tasks, and an id "
            f"pattern without the letter suffix matches `T-01` inside `T-01a` — "
            f"so the check would either report the wrong task or, because T-01 "
            f"has a row, report nothing at all.",
        )

        print(
            f"plan traceability: {len(table_ids(self.plan))} rows, "
            f"{len(mentioned_ids(self.plan))} ids named, "
            f"{len(commit_ids(self.subjects))} ids in {len(self.subjects)} commits; "
            f"removing a row and fabricating a mention are both reported"
        )

    def test_every_todo_tag_names_a_row(self):
        """T-39, direction three. Would have caught T-38's absence.

        `TODO(T-38)` was written before T-38 had a row, and the tags were the
        only place that work was recorded. This proves each tag's id resolves to
        a row — and nothing more; see the module docstring for why the stronger
        claim is not mechanised here.
        """
        rows = table_ids(self.plan)
        problems = dangling_tags(todo_tags(), rows)
        self.assertEqual(
            [],
            problems,
            "these TODO tags name a task id that docs/migration/"
            f"{PLAN.name} has no row for:\n"
            + "\n".join(
                f"  {path}:{line}  {tag}  -> {task}: no row in PLAN.md"
                for tag, task, path, line in problems
            )
            + "\n\nA tag is a promise that the work belongs to a task. An id with "
            "no row is a promise to nobody: the tag is the only place that work is "
            "recorded, which is how T-38 stayed invisible until 84d367e. Write the "
            "row, or retag onto the task that owns the work.\n\n"
            "  PROVES     every id written in a `TODO(T-nn)` resolves to a row.\n"
            "  DOES NOT   prove that row's declared scope contains this work.\n\n"
            "The second is a judgement over two passages of prose — the row's scope "
            "string and the code the tag sits in — and it is deliberately not "
            "mechanised: a check claiming to make it would be #69's shape. So a "
            "green run does not mean the tags are right; it means each names a real "
            "task. Rot where both ids resolve — installer work tagged T-13, cover "
            "work tagged T-11 (#79) — passes exactly as a correct tag does.",
        )

        # The report half, which is what makes the semantic half tractable: six
        # ids can be compared against their scopes by hand where 32 tags cannot.
        tags = todo_tags()
        per_id = {}
        for _, task, path, _ in tags:
            per_id.setdefault(task, []).append(path)
        histogram = "  ".join(
            f"{task} {len(paths)}" for task, paths in sorted(per_id.items())
        )
        print(
            f"todo tags: {len(tags)} tags over {len(per_id)} ids in "
            f"{len({t[2] for t in tags})} files\n"
            f"  by id — names a row is not owns the work; compare each against its "
            f"scope:\n    {histogram}"
        )
        # Named individually, because a deferred row is still a defect on disk.
        for (doc, first), reason in sorted(MALFORMED_ROWS.items()):
            print(f"  malformed row, deferred: {doc} row {first} — {reason[:80]}…")

    def test_the_tag_check_notices_an_undefined_id(self):
        """The check above, driven over an id it must reject (#26).

        A source-reading check that has only ever passed is one whose failure
        branch nobody has watched. This drives the same `dangling_tags` the test
        above uses, over a fabricated tag list, and requires the real tags to
        come back clean in the same run — so the comparison is not two runs of
        different code.
        """
        rows = table_ids(self.plan)
        real = todo_tags()
        self.assertEqual(
            [], dangling_tags(real, rows), "this test needs a clean baseline to mutate from"
        )

        fabricated = real + [
            ("TODO(T-99)", "T-99", "crates/app/src/main.rs", 1),
            ("TODO(T-98)", "T-98", "crates/app/src/view/library.rs", 2),
        ]
        reported = dangling_tags(fabricated, rows)
        self.assertEqual(
            ["T-98", "T-99"],
            sorted(task for _, task, _, _ in reported),
            f"a fabricated tag was not reported. Everything reported: {reported}. "
            "The direction is source -> plan: a `TODO(T-nn)` naming an id with no "
            "row is exactly T-38's shape, and the tag is the only place that work "
            "is written down.",
        )
        # And the site survives into the report, because an id alone does not
        # tell a reader where to look.
        self.assertTrue(
            any(path == "crates/app/src/main.rs" and line == 1 for _, _, path, line in reported),
            f"the fabricated tag was reported without its site; got {reported}",
        )

    def test_every_table_row_has_its_headers_cell_count(self):
        """#84: a split row renders anyway, so nothing notices it was split."""
        documents = ((PLAN.name, self.plan), (FINDINGS.name, self.findings))
        problems = [row for doc, text in documents for row in malformed_rows(doc, text)]
        live = [row for doc, text in documents for row in live_malformed_rows(doc, text)]

        self.assertEqual(
            [],
            [(doc, line, expected, got) for doc, line, expected, got, _, _ in live],
            "these table rows have a different number of cells than their own "
            "header, which means either a literal `|` split a cell or a cell was "
            "written past the last column:\n"
            + "\n".join(
                f"  {doc}:{line}  {got} cells, header has {expected}  (starts {first!r})"
                for doc, line, expected, got, first, _ in live
            )
            + "\n\nThe row still *renders*, which is why this was committed green "
            "three times (#84): a split cell is invisible in the output. Rewrite "
            "the cell with the pipe escaped as `\\|`, or drop the extra cell, or "
            "add the column to the table's header — whichever the row was meant to "
            "be. Do not repair it from inside this test: the row is the owner's to "
            "rewrite, and a test that edited the document it checks could report a "
            "tree it had just made clean.",
        )
        # Every deferred row is accounted for by a named entry, so a deferral
        # cannot cover more than it claims. Stated over the *set difference*
        # rather than a count of entries, because one misheaded-table entry
        # covers many rows and a total alone would not say which.
        named = {
            (row[0], row[4]) for row in problems
        } & set(MALFORMED_ROWS)
        named |= {
            (doc, table)
            for doc, table in {(row[0], row[5]) for row in problems}
            if (doc, table) in MISHEADED_TABLES
        }
        self.assertEqual(
            set(MALFORMED_ROWS) | set(MISHEADED_TABLES),
            named,
            "a deferral entry names something that is not a malformed row or a "
            "misheaded table on this tree — it is covering nothing, or covering "
            "something it does not describe",
        )
        for (doc, table), reason in sorted(MISHEADED_TABLES.items()):
            rows = [row for row in problems if row[0] == doc and row[5] == table]
            print(
                f"  misheaded table, deferred: {doc} table starting at row "
                f"{table!r} — {len(rows)} rows disagree with their header. "
                f"{reason[:100]}…"
            )

    def test_the_malformed_row_list_is_not_stale(self):
        """Deferrals are checked, so the list cannot outlive the defect."""
        known = {
            (doc, first)
            for doc, text in ((PLAN.name, self.plan), (FINDINGS.name, self.findings))
            for doc, _, _, _, first, _ in malformed_rows(doc, text)
        }
        stale = sorted(entry for entry in MALFORMED_ROWS if entry not in known)
        self.assertEqual(
            [],
            stale,
            f"these MALFORMED_ROWS entries no longer describe a malformed row: "
            f"{stale}. The row was fixed, or the table was rewritten — either way, "
            f"delete the entry. A deferral list that is not checked is how a "
            f"deferral list turns into a place where problems are forgotten.",
        )

    def test_the_misheaded_table_list_is_not_stale(self):
        """A whole-table deferral expires the same way a row's does.

        The check is that the entry still names a table with rows disagreeing
        with its own header — so fixing the header at `PLAN.md:361` fails this
        test until the entry is deleted, rather than leaving a deferral in place
        over a table that no longer needs one.
        """
        # Every row of the named table, not just the malformed ones: the entry
        # claims "this header is wrong for this table", and a table reduced to
        # one disagreeing row is still that.
        live = {
            (doc, table)
            for doc, text in ((PLAN.name, self.plan), (FINDINGS.name, self.findings))
            for doc, _, _, _, _, table in malformed_rows(doc, text)
        }
        stale = sorted(entry for entry in MISHEADED_TABLES if entry not in live)
        self.assertEqual(
            [],
            stale,
            f"these MISHEADED_TABLES entries no longer name a table whose rows "
            f"disagree with its header: {stale}. The header was fixed — delete the "
            f"entry, and the row check above is then asserting over that table "
            f"for the first time.",
        )

    def test_every_row_run_is_immediately_headed(self):
        """#93: a run of well-formed rows with no header above it is not a table.

        The check above counts cells against a header. This one asks the question
        that has to come first — whether there *is* a header, immediately above —
        and it exists because the answer was no for 56 lines of these two
        documents (`PLAN.md:361`, `VERIFY-FINDINGS.md:79`, `:432`, `:479`) while
        every one of those rows had a perfect cell count. Without it this file
        would print "17 tables over 2 documents" and assert both documents clean
        while the second half of the task list rendered as prose.
        """
        problems = [
            run
            for doc, text in ((PLAN.name, self.plan), (FINDINGS.name, self.findings))
            for run in headerless_runs(doc, text)
        ]
        deferred, live = [], []
        for run in problems:
            (deferred if (run[0], run[3]) in MALFORMED_RUNS else live).append(run)

        self.assertEqual(
            [],
            live,
            "these runs of `|`-lines are not preceded by a header row and a "
            "delimiter row, so no GFM renderer shows them as a table — they are "
            "paragraphs:\n"
            + "\n".join(
                f"  {doc}:{line}  {n} lines, first cell {first!r}"
                + (f", {n_ids} of them rows with an id" if n_ids else "")
                for doc, line, n, first, n_ids in live
            )
            + "\n\nA blank line or any other non-row line ends a GFM table for "
            "good; it does not resume, and it does not matter that a header "
            "exists further up. Either give the run its own header and `|---|` "
            "delimiter row, or move the prose that split it below the rows. Add "
            "the header rather than deleting the rows: the rows are the content, "
            "and every one of them is currently rendered as text.",
        )
        self.assertEqual(
            len(MALFORMED_RUNS),
            len(deferred),
            "a deferred run is no longer reported",
        )
        # Stated in the output, because the assertion above is green — and a
        # green run is exactly the thing a reader trusts without reading.
        print(
            "table shape: "
            + "; ".join(
                f"{doc} {len(tables(text))} of {len(pipe_runs(text))} pipe-runs are "
                f"tables and its {len(id_runs(text))} id-bearing runs "
                f"({sum(n for _, n in id_runs(text))} rows) are headed"
                for doc, text in ((PLAN.name, self.plan), (FINDINGS.name, self.findings))
            )
        )

    def test_the_headerless_run_check_notices_a_removed_header(self):
        """The #93 detector, driven over the exact state it was written for.

        The mutation is the real defect, restored: the header and delimiter rows
        the lead inserted at `PLAN.md:361` and `:362` are removed in memory, so
        the run of T-12…T-40 rows is again what it was before `7d8176e` — a
        paragraph of perfect rows. The check must name the run, and it must name
        it by what is *in* the run: 29 ids, the second half of the task table.

        This is the DoD for #93. A structural check that has never been shown to
        fail on the state it was written against is a check nobody has watched
        work, which is the failure this whole file keeps coming back to.
        """
        before = [
            run
            for doc, text in ((PLAN.name, self.plan), (FINDINGS.name, self.findings))
            for run in headerless_runs(doc, text)
        ]
        self.assertEqual(
            [],
            before,
            f"this test needs a fully headed baseline to remove a header from; "
            f"every run is already governed, and these are not: {before}",
        )

        rows = self.plan.splitlines()
        # Find the header by its *position*, not its text: at this sha the header
        # at `:361` is wrong for its table (#94), and this test must keep working
        # when the lead replaces it with the right one.
        first_row = next(
            n for n, line in enumerate(rows, start=1) if line.startswith("| T-12 |")
        )
        self.assertTrue(
            is_delimiter(rows[first_row - 2]) and rows[first_row - 3].startswith("|"),
            f"the task table's second half is not headed at {PLAN.name}:"
            f"{first_row - 2}; this test removes that header and needs it there",
        )
        mutated = "\n".join(rows[: first_row - 3] + rows[first_row - 1 :])

        found = headerless_runs(PLAN.name, mutated)
        self.assertEqual(
            1,
            len(found),
            f"removing the header above the T-12 run did not leave exactly one "
            f"unheaded run. Reported: {found}",
        )
        doc, line, n_lines, first, n_ids = found[0]
        self.assertEqual("T-12", first, f"the unheaded run was reported as {found}")
        # Two rows were deleted above it, so the run starts two lines earlier.
        self.assertEqual(first_row - 2, line)
        self.assertEqual(
            29,
            n_lines,
            f"the run was counted as {n_lines} lines; T-12…T-40 is 29 rows, and "
            f"a report that understates the run is how a 29-row loss reads as a "
            f"one-row one",
        )
        self.assertEqual(
            29,
            n_ids,
            f"the run was counted as {n_ids} id-bearing rows; every one of the 29 "
            f"carries a `T-nn`, so the report understates what the paragraph costs",
        )

    def test_the_table_check_notices_a_split_cell(self):
        """#84's detector, driven over a row it must reject (#26).

        The mutation is the defect itself: a real Rust closure with a literal
        `|`, written into a task-table row, which is what `9eaf11d` shipped and
        what a green gate did not see.
        """
        clean = live_malformed_rows(PLAN.name, self.plan)
        self.assertEqual(
            [],
            clean,
            f"this test needs a plan whose live defects are all deferred, so the "
            f"mutation is the only thing it can be reporting; got {clean}",
        )
        # The split row goes into the *first* half of the task table, whose header
        # is right — so what this test reports is the pipe and not #94's header.
        anchor = next(
            line for line in self.plan.splitlines() if line.startswith("| T-08 |")
        )
        split_row = (
            "| T-99 | `app`: probe | UX | `DEFAULT_TOGGLES.iter().map(|(key, _, _)| "
            "*key)` |"
        )
        mutated = self.plan.replace(anchor, anchor + "\n" + split_row, 1)
        self.assertNotEqual(mutated, self.plan, "the mutation did not change the plan")

        found = live_malformed_rows(PLAN.name, mutated)
        self.assertEqual(
            [("T-99", 6)],
            [(first, got) for _, _, _, got, first, _ in found],
            f"a row whose cell contains an unescaped `|` was not the only live "
            f"defect reported. Reported: {found}. The literal pipe in "
            f"`map(|(key, _, _)| *key)` splits the cell into 6, the row still "
            f"renders, and that is #84 — the defect this check exists for and the "
            f"one it is reachable at only if the count is taken after removing the "
            f"edge pipes.",
        )
        print(
            f"table check: {sum(len(tables(t)) for t in (self.plan, self.findings))} "
            f"tables over 2 documents, "
            f"{sum(len(rows) for t in (self.plan, self.findings) for _, _, rows in tables(t))} "
            f"rows in them; the mutated plan's only live defect is "
            f"{[first for _, _, _, _, first, _ in found]} — the fabricated row whose "
            f"cell contains `map(|(key, _, _)| *key)`"
        )


if __name__ == "__main__":
    unittest.main()
