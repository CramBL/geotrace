"""Forbid em-dash / box-drawing section header comments in source files.

Covers Rust, Python, Just, CMake, C, and C++ files.

Exempt a line with a qa-allow comment on the same line:
    // ── Section ── // [qa-allow-check-em-dash, reason = "why"]
Multiple checks may share one comment:
    // ── Section ── // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "why"]
"""

import re
from itertools import chain
from pathlib import Path

from qa._allow import is_exempt
from qa._check import (
    Violation,
    c_family_files,
    hash_comment_files,
    repo_root,
    rs_files,
    run_check,
)

CHECK = "check-em-dash"
_EMDASH = re.compile(r"^\s*(//|#|/\*).*[─═]")
_NOTE = [
    "em-dash characters are ASCII-art decoration, not documentation;",
    "prefer plain comments attached to code, or module/impl structure",
]
_HELP = ["exempt a line by appending:"]


def _collect(root: Path) -> list[Violation]:
    violations: list[Violation] = []
    for path in chain(rs_files(root), hash_comment_files(root), c_family_files(root)):
        for lineno, raw in enumerate(path.read_text(errors="replace").splitlines(), 1):
            if _EMDASH.match(raw) and not is_exempt(raw, CHECK):
                violations.append((path, lineno, raw.strip()))
    return violations


def main() -> None:
    run_check(
        CHECK,
        "em-dash / box-drawing section header comments found",
        _collect(repo_root()),
        _NOTE,
        _HELP,
    )
