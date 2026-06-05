"""Forbid floating comments (blank line both before and after) in Rust and C/C++ source files.

A floating comment is a // or /* */ comment with a blank line above it and a blank
line below it — it is a section label detached from its code.

Fix by removing the blank line between the comment and the code below it, converting
to ///, or exempting with a qa-allow comment on the same line:
    // Floating label // [qa-allow-check-floating-comments, reason = "why"]
Multiple checks may share one comment:
    // Label // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "why"]
"""

import re
from pathlib import Path

from qa._allow import is_exempt
from qa._check import Violation, c_family_files, rs_files, run_check

CHECK = "check-floating-comments"
_RS_COMMENT = re.compile(r"^\s*//[^/!]")
_C_LINE_COMMENT = re.compile(r"^\s*//[^/]")
_C_BLOCK_COMMENT = re.compile(r"^\s*/\*[^*].*\*/\s*$")
_NOTE = ["this comment is surrounded by blank lines and not attached to any code"]
_HELP = [
    "remove the blank line between the comment and the code below it,",
    "convert to ///, or exempt with:",
]


def _is_floating(lines: list[str], i: int) -> bool:
    prev_blank = i == 0 or not lines[i - 1].strip()
    next_blank = i + 1 >= len(lines) or not lines[i + 1].strip()
    return prev_blank and next_blank


def _collect(root: Path) -> list[Violation]:
    violations: list[Violation] = []

    for path in rs_files(root):
        lines = path.read_text(errors="replace").splitlines()
        for i, raw in enumerate(lines):
            if _RS_COMMENT.match(raw) and _is_floating(lines, i) and not is_exempt(raw, CHECK):
                violations.append((path, i + 1, raw.strip()))

    for path in c_family_files(root):
        lines = path.read_text(errors="replace").splitlines()
        for i, raw in enumerate(lines):
            is_comment = _C_LINE_COMMENT.match(raw) or _C_BLOCK_COMMENT.match(raw)
            if is_comment and _is_floating(lines, i) and not is_exempt(raw, CHECK):
                violations.append((path, i + 1, raw.strip()))

    return violations


def main() -> None:
    run_check(CHECK, "floating comments found", _collect(Path(".")), _NOTE, _HELP)
