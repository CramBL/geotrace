"""Cap how long a commit message body may run.

A body states what changed and why. It is not a second reading of the diff, and
a message that walks a change file by file pushes the reason out of sight.

The ceiling comes from the repository's own history. Over the 511 commits trunk
held when this check landed, the median body is 5 lines, the 95th percentile 19,
and 21 bodies run past 20 lines. Three run past 40: 42, 54 and 55 lines. A
message fails this check only by being an outlier in the repository it lands in:
a ceiling of 40 lines passes 508 of the 511. The message this check was written
for ran 124 lines.

The count covers every line after the subject, including a blank line between
two paragraphs. It leaves out a blank line at the start or the end of the body.
A `fixup!` commit is never counted: its message never reaches the branch. An
`amend!` commit is counted on the replacement message it writes over its
target's.
"""

import argparse
import sys
from pathlib import Path

from qa._check import Commit, commits_in, repo_root, report_locations

CHECK = "check-commit-length"

MAX_BODY_LINES = 40

_DEFAULT_RANGE = "origin/trunk..HEAD"


def body_line_count(commit: Commit) -> int:
    body = commit.message_to_land().partition("\n")[2].strip("\n")
    return len(body.splitlines()) if body.strip() else 0


def over_ceiling(commits: list[Commit]) -> list[tuple[Commit, int]]:
    counted = [(commit, body_line_count(commit)) for commit in commits]
    return [(commit, lines) for commit, lines in counted if lines > MAX_BODY_LINES]


_NOTE = [
    "a body states what changed and why, not what the diff already shows.",
    f"508 of the 511 commits trunk held when this check landed run under the {MAX_BODY_LINES}",
    "line ceiling, and the median body runs 5 lines.",
]
_HELP = [
    "cut the body to the change and its reason.",
    "a file-by-file account of the change is in the diff already.",
]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=repo_root())
    parser.add_argument("range", nargs="?", default=_DEFAULT_RANGE)
    args = parser.parse_args()

    root: Path = args.repo_root.resolve()
    found = over_ceiling(commits_in(root, args.range))
    if not found:
        return
    report_locations(
        CHECK,
        f"commit message body over {MAX_BODY_LINES} lines",
        [
            (f"commit {commit.hash} {commit.subject}", f"body of {lines} lines")
            for commit, lines in found
        ],
        _NOTE,
        _HELP,
    )
    sys.exit(1)
