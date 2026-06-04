"""Forbid floating // comments (blank line both before and after) in Rust source files.

A floating comment is a regular // comment with a blank line above it and a blank
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
from qa._check import Violation, rs_files, run_check

CHECK = "check-floating-comments"
_COMMENT = re.compile(r"^\s*//[^/!]")
_NOTE = ["this comment is surrounded by blank lines and not attached to any code"]
_HELP = [
    "remove the blank line between the comment and the code below it,",
    "convert to ///, or exempt with:",
]


def _collect(root: Path) -> list[Violation]:
    violations: list[Violation] = []
    for path in rs_files(root):
        lines = path.read_text(errors="replace").splitlines()
        for i, raw in enumerate(lines):
            if not _COMMENT.match(raw):
                continue
            prev_blank = i == 0 or not lines[i - 1].strip()
            next_blank = i + 1 >= len(lines) or not lines[i + 1].strip()
            if prev_blank and next_blank and not is_exempt(raw, CHECK):
                violations.append((path, i + 1, raw.strip()))
    return violations


def main() -> None:
    run_check(CHECK, "floating // comments found", _collect(Path(".")), _NOTE, _HELP)
