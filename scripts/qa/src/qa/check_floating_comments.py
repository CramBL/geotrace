"""Forbid floating comments in Rust, C/C++, and YAML source files.

A floating comment is a run of consecutive comment lines with a blank line above
the run and a blank line below it: a section label detached from its code, whether
it's one line or a whole paragraph of them.

Fix by removing the blank line between the comment and the code below it, converting
to /// (Rust), or exempting with a qa-allow comment on the run's first line:
    // Floating label // [qa-allow-check-floating-comments, reason = "why"]
    # Floating label # [qa-allow-check-floating-comments, reason = "why"]
One comment may exempt several checks at once, as `qa._allow` documents.
"""

import re
import sys
from collections.abc import Callable
from pathlib import Path

from qa._allow import is_exempt
from qa._check import Check, Violation, c_family_files, repo_root, rs_files, run_check, yaml_files

CHECK = "check-floating-comments"
# `(?:[^/!]|$)` also matches bare `//` continuation lines (no trailing text),
# which are common inside multi-line comment paragraphs.
_RS_COMMENT = re.compile(r"^\s*//(?:[^/!]|$)")
_C_LINE_COMMENT = re.compile(r"^\s*//(?:[^/]|$)")
_C_BLOCK_COMMENT = re.compile(r"^\s*/\*[^*].*\*/\s*$")
_YAML_COMMENT = re.compile(r"^\s*#")
_NOTE = ["this comment is surrounded by blank lines and not attached to any code"]
_HELP = [
    "remove the blank line between the comment and the content below it,",
    "convert to /// (Rust), or exempt with:",
]


def _comment_runs(lines: list[str], is_comment: Callable[[str], bool]) -> list[tuple[int, int]]:
    """Return `(start, end)` inclusive index ranges of consecutive comment lines."""
    runs: list[tuple[int, int]] = []
    i = 0
    while i < len(lines):
        if is_comment(lines[i]):
            start = i
            while i < len(lines) and is_comment(lines[i]):
                i += 1
            runs.append((start, i - 1))
        else:
            i += 1
    return runs


def _is_floating_run(lines: list[str], start: int, end: int) -> bool:
    prev_blank = start == 0 or not lines[start - 1].strip()
    next_blank = end + 1 >= len(lines) or not lines[end + 1].strip()
    return prev_blank and next_blank


def _collect(root: Path) -> list[Violation]:
    violations: list[Violation] = []

    for path in rs_files(root):
        lines = path.read_text(errors="replace").splitlines()
        for start, end in _comment_runs(lines, lambda line: bool(_RS_COMMENT.match(line))):
            if _is_floating_run(lines, start, end) and not is_exempt(lines[start], CHECK):
                violations.append((path, start + 1, lines[start].strip()))

    for path in c_family_files(root):
        lines = path.read_text(errors="replace").splitlines()

        def _is_c_comment(line: str) -> bool:
            return bool(_C_LINE_COMMENT.match(line) or _C_BLOCK_COMMENT.match(line))

        for start, end in _comment_runs(lines, _is_c_comment):
            if _is_floating_run(lines, start, end) and not is_exempt(lines[start], CHECK):
                violations.append((path, start + 1, lines[start].strip()))

    for path in yaml_files(root):
        lines = path.read_text(errors="replace").splitlines()
        for start, end in _comment_runs(lines, lambda line: bool(_YAML_COMMENT.match(line))):
            if _is_floating_run(lines, start, end) and not is_exempt(lines[start], CHECK):
                violations.append((path, start + 1, lines[start].strip()))

    return violations


DEFINITION = Check(
    name=CHECK,
    title="floating comments found",
    collect=_collect,
    note=_NOTE,
    help=_HELP,
)


def main() -> None:
    if run_check(DEFINITION, repo_root()):
        sys.exit(1)
