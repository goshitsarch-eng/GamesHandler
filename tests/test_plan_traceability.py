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
"""

import re
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PLAN = ROOT / "docs" / "migration" / "PLAN.md"

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

#: Task ids PLAN.md names but never defines, with the finding that owns each.
#:
#: **A deferral, not an exemption.** An entry that stops being a dangling
#: mention — because the row was written, or the mention was removed — fails
#: [`PlanTraceabilityTests.test_the_deferral_list_is_not_stale`], so this cannot
#: become a list of things nobody looks at again.
#:
#: EMPTY right now, which is where it should stay. If you are here to add one,
#: the cheaper fix is almost always to write the missing row.
UNROUTED = {
    "T-63": (
        "#71 (second instance). T-04's row names it as the owner of the `.desktop` "
        "writer — `T-63` is P-71, the desktop-shortcut writer, and T-04's row "
        "sequences the whole critical path around it. No row defines it: `git grep "
        "T-63` over tracked files returns PLAN.md:347 and nothing else. It is "
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


class PlanTraceabilityTests(unittest.TestCase):
    def setUp(self):
        self.plan = PLAN.read_text(encoding="utf-8")
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


if __name__ == "__main__":
    unittest.main()
