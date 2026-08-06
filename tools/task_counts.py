#!/usr/bin/env python3
"""Keep the summary table in docs/TASKS.md honest.

`TASKS.md` is the project's source of truth for status, and it carries the same
information twice: a checkbox per task, and a summary table of counts. Two
representations of one fact drift, and hand-maintained counts drift on the very
first change someone forgets to mirror — at which point the summary quietly lies
and nobody notices, because nobody recounts 120 checkboxes to check.

This derives the table from the checkboxes so the counts cannot be wrong.

    python3 tools/task_counts.py            # rewrite the table
    python3 tools/task_counts.py --check    # fail if it is out of date (CI)
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

TASKS = Path(__file__).resolve().parent.parent / "docs" / "TASKS.md"

# Milestones counted toward the v1 total; the rest are post-v1.
V1_MILESTONES = ("M0", "M1", "M2", "M3", "M4", "M5", "M6", "M7")


def count_by_milestone(text: str) -> dict[str, tuple[int, int]]:
    """Checked and total tasks per milestone, read from the section bodies."""
    counts: dict[str, tuple[int, int]] = {}
    for section in text.split("\n## ")[1:]:
        heading = re.match(r"(M\d)\b", section)
        if not heading:
            continue
        done = len(re.findall(r"^- \[x\]", section, re.M))
        pending = len(re.findall(r"^- \[ \]", section, re.M))
        counts[heading.group(1)] = (done, done + pending)
    return counts


def rewrite(text: str, counts: dict[str, tuple[int, int]]) -> str:
    def milestone_row(match: re.Match[str]) -> str:
        name = match.group(1)
        if name not in counts:
            return match.group(0)
        done, total = counts[name]
        return re.sub(r"\|\s*\d+\s*\|\s*\d+\s*\|", f"| {done} | {total} |", match.group(0))

    text = re.sub(r"\| \*\*(M\d)\*\* \|[^\n]*\|", milestone_row, text)

    v1_done = sum(counts[m][0] for m in counts if m in V1_MILESTONES)
    v1_total = sum(counts[m][1] for m in counts if m in V1_MILESTONES)
    grand_total = sum(total for _, total in counts.values())

    text = re.sub(
        r"\| — \| \*\*v1 total\*\* \| `1\.0\.0` \| \*\*\d+\*\* \| \*\*\d+\*\* \| \|",
        f"| — | **v1 total** | `1.0.0` | **{v1_done}** | **{v1_total}** | |",
        text,
    )
    return re.sub(
        r"\*\*Overall:\*\* \d+ / \d+ tasks done \(\d+%\)",
        f"**Overall:** {v1_done} / {grand_total} tasks done "
        f"({round(100 * v1_done / grand_total)}%)",
        text,
    )


def main() -> int:
    original = TASKS.read_text()
    updated = rewrite(original, count_by_milestone(original))

    if "--check" in sys.argv:
        if original == updated:
            print("docs/TASKS.md counts match the checkboxes")
            return 0
        print(
            "docs/TASKS.md summary table is out of date.\n"
            "Run: python3 tools/task_counts.py",
            file=sys.stderr,
        )
        return 1

    if original == updated:
        print("already up to date")
    else:
        TASKS.write_text(updated)
        print("updated docs/TASKS.md")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
